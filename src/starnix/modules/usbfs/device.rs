// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use fidl::endpoints::{SynchronousProxy, create_endpoints};
use fidl_fuchsia_hardware_usb as fhardware_usb;
use fidl_fuchsia_hardware_usb_descriptor as fusb_descriptor;
use fidl_fuchsia_hardware_usb_endpoint as fusb_endpoint;
use fidl_fuchsia_hardware_usb_request as fusb_request;
use starnix_core::mm::{MemoryAccessor, MemoryAccessorExt};
use starnix_core::task::dynamic_thread_spawner::SpawnRequestBuilder;
use starnix_core::task::{
    CurrentTask, EventHandler, SignalHandler, SignalHandlerInner, TaskFlags, Waiter, WaiterOptions,
};
use starnix_core::vfs::buffers::{InputBuffer, OutputBuffer};
use starnix_core::vfs::{FileObject, FileOps, fileops_impl_noop_sync, fileops_impl_seekable};
use starnix_logging::track_stub;
use starnix_sync::{
    LockDepMutex, UsbfsControlSlotLock, UsbfsDeviceStateLock, UsbfsEndpointStateLock,
    UsbfsEndpointTransferLock, UsbfsEndpointsLock,
};
use starnix_syscalls::{SUCCESS, SyscallArg, SyscallResult};
use starnix_uapi::errors::{EINTR, ENODEV, ENOENT, ESHUTDOWN, ETIMEDOUT, Errno};
use starnix_uapi::signals::SigSet;
use starnix_uapi::user_address::{ArchSpecific, MultiArchUserRef, UserAddress, UserRef};
use starnix_uapi::{
    _IOC_DIRSHIFT, _IOC_SIZEMASK, _IOC_SIZESHIFT, _IOC_WRITE, USB_CLASS_PRINTER, USB_DIR_IN,
    USB_DIR_OUT, USB_DT_CONFIG, USB_DT_ENDPOINT, USB_DT_ENDPOINT_SIZE, USB_DT_INTERFACE,
    USB_DT_INTERFACE_ASSOCIATION, USB_DT_SS_ENDPOINT_COMP, USB_ENDPOINT_HALT,
    USB_ENDPOINT_MAXP_MASK, USB_ENDPOINT_NUMBER_MASK, USB_ENDPOINT_XFER_BULK,
    USB_ENDPOINT_XFER_INT, USB_ENDPOINT_XFERTYPE_MASK, USB_RECIP_ENDPOINT, USB_RECIP_INTERFACE,
    USB_RECIP_MASK, USB_REQ_CLEAR_FEATURE, USB_TYPE_CLASS, USB_TYPE_MASK, USB_TYPE_STANDARD,
    USB_TYPE_VENDOR, USBDEVFS_BULK, USBDEVFS_CLAIMINTERFACE, USBDEVFS_CLEAR_HALT, USBDEVFS_CONNECT,
    USBDEVFS_CONNECTINFO, USBDEVFS_CONTROL, USBDEVFS_DISCONNECT, USBDEVFS_DISCONNECT_CLAIM,
    USBDEVFS_DISCONNECT_CLAIM_EXCEPT_DRIVER, USBDEVFS_DISCONNECT_CLAIM_IF_DRIVER,
    USBDEVFS_GET_CAPABILITIES, USBDEVFS_GET_SPEED, USBDEVFS_GETDRIVER, USBDEVFS_IOCTL,
    USBDEVFS_RELEASEINTERFACE, USBDEVFS_RESET, USBDEVFS_SETCONFIGURATION, USBDEVFS_SETINTERFACE,
    errno, error, uapi, usb_config_descriptor, usb_device_descriptor,
    usb_device_speed_USB_SPEED_FULL, usb_device_speed_USB_SPEED_HIGH,
    usb_device_speed_USB_SPEED_LOW, usb_device_speed_USB_SPEED_SUPER,
    usb_device_speed_USB_SPEED_SUPER_PLUS, usb_device_speed_USB_SPEED_UNKNOWN,
    usb_device_speed_USB_SPEED_WIRELESS, usb_interface_descriptor, usb_ss_ep_comp_descriptor,
    usbdevfs_bulktransfer, usbdevfs_connectinfo, usbdevfs_ctrltransfer, usbdevfs_disconnect_claim,
    usbdevfs_getdriver, usbdevfs_ioctl, usbdevfs_setinterface,
};
use std::cmp::min;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::sync_channel;
use zerocopy::FromBytes;

type UsbdevfsCtrltransferPtr =
    MultiArchUserRef<usbdevfs_ctrltransfer, uapi::arch32::usbdevfs_ctrltransfer>;
type UsbdevfsBulktransferPtr =
    MultiArchUserRef<usbdevfs_bulktransfer, uapi::arch32::usbdevfs_bulktransfer>;
type UsbdevfsIoctlPtr = MultiArchUserRef<usbdevfs_ioctl, uapi::arch32::usbdevfs_ioctl>;

pub(crate) fn canonicalize_ioctl_request(is_arch32: bool, request: u32) -> u32 {
    if is_arch32 {
        match request {
            uapi::arch32::USBDEVFS_CONTROL => uapi::USBDEVFS_CONTROL,
            uapi::arch32::USBDEVFS_BULK => uapi::USBDEVFS_BULK,
            uapi::arch32::USBDEVFS_DISCSIGNAL => uapi::USBDEVFS_DISCSIGNAL,
            uapi::arch32::USBDEVFS_SUBMITURB => uapi::USBDEVFS_SUBMITURB,
            uapi::arch32::USBDEVFS_REAPURB => uapi::USBDEVFS_REAPURB,
            uapi::arch32::USBDEVFS_REAPURBNDELAY => uapi::USBDEVFS_REAPURBNDELAY,
            uapi::arch32::USBDEVFS_IOCTL => uapi::USBDEVFS_IOCTL,
            _ => request,
        }
    } else {
        request
    }
}

pub(crate) const USB_SPEED_UNKNOWN: u32 = usb_device_speed_USB_SPEED_UNKNOWN;
pub(crate) const USB_SPEED_LOW: u32 = usb_device_speed_USB_SPEED_LOW;
pub(crate) const USB_SPEED_FULL: u32 = usb_device_speed_USB_SPEED_FULL;
pub(crate) const USB_SPEED_HIGH: u32 = usb_device_speed_USB_SPEED_HIGH;
pub(crate) const USB_SPEED_WIRELESS: u32 = usb_device_speed_USB_SPEED_WIRELESS;
pub(crate) const USB_SPEED_SUPER: u32 = usb_device_speed_USB_SPEED_SUPER;
pub(crate) const USB_SPEED_SUPER_PLUS: u32 = usb_device_speed_USB_SPEED_SUPER_PLUS;

pub(crate) const MAX_USBDEVFS_CONTROL_SIZE: usize = 4096;

/// Approximate size of the Linux `struct urb` on 64-bit architectures (184 bytes without vendor
/// padding). usbfs charges it to the memory used by each transfer, on top of the data buffer.
const URB_SIZE: usize = 184;

/// Memory available to a usbfs transfer (the default `usbfs_memory_mb` of 16 MiB).
const USBFS_MEMORY_LIMIT: usize = 16 * 1024 * 1024;

/// Largest data length of a transfer that fits in `USBFS_MEMORY_LIMIT`.
pub(crate) const MAX_USBDEVFS_BULK_SIZE: usize = USBFS_MEMORY_LIMIT - URB_SIZE;

/// Data lengths of `USBDEVFS_BULK` transfers from this value on are invalid (`EINVAL`), before
/// the memory limit is checked.
pub(crate) const INVALID_USBDEVFS_BULK_SIZE: usize = i32::MAX as usize - URB_SIZE;

const ENDPOINT_VMO_SIZE: u64 = 65536;

/// Maximum number of retired VMOs of an endpoint, i.e. VMOs whose cancelled requests did not
/// complete in time. Transfers on the endpoint fail with `EIO` while this many VMOs still wait
/// for completions.
pub(crate) const MAX_RETIRED_VMOS: usize = 4;

/// Maximum number of interfaces supported by the `u64` interface claim bitset.
const MAX_USB_INTERFACES: u32 = u64::BITS;

/// Default upper bound on the time spent waiting for the completions of cancelled requests
/// before the endpoint VMO is retired.
pub(crate) const DEFAULT_CANCEL_DRAIN_TIMEOUT: zx::MonotonicDuration =
    zx::MonotonicDuration::from_seconds(1);

/// Timeout of the `CLEAR_FEATURE(ENDPOINT_HALT)` control transfers issued by usbfs (matching
/// Linux `USB_CTRL_SET_TIMEOUT`).
const CLEAR_HALT_TIMEOUT: zx::MonotonicDuration = zx::MonotonicDuration::from_millis(5000);

/// Driver name reported for interfaces claimed through usbfs. Starnix never binds kernel drivers
/// to USB interfaces, so usbfs is the only possible driver.
const USBFS_DRIVER_NAME: &[u8] = b"usbfs";

/// `bmRequestType` and `bRequest` of the printer class GET_DEVICE_ID request.
const USB_PRINTER_GET_DEVICE_ID_REQUEST_TYPE: u32 =
    USB_DIR_IN | USB_TYPE_CLASS | USB_RECIP_INTERFACE;
const USB_PRINTER_GET_DEVICE_ID: u8 = 0;

/// Offsets of the fields of a standard endpoint descriptor (USB 2.0 Table 9-13). The uapi
/// `usb_endpoint_descriptor` has the 9-byte audio layout, so the 7-byte descriptor is read field
/// by field.
const ENDPOINT_DESC_ADDRESS: usize = 2;
const ENDPOINT_DESC_ATTRIBUTES: usize = 3;
const ENDPOINT_DESC_MAX_PACKET_SIZE: usize = 4;
const ENDPOINT_DESC_INTERVAL: usize = 6;

/// Validated USB interface number in `0..MAX_USB_INTERFACES` (`0..64`),
/// suitable for indexing into a `u64` interface claim bitset.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
struct Ifno(u8);

impl Ifno {
    fn as_u8(self) -> u8 {
        self.0
    }
}

impl TryFrom<u32> for Ifno {
    type Error = Errno;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        if value >= MAX_USB_INTERFACES { error!(EINVAL) } else { Ok(Self(value as u8)) }
    }
}

impl From<Ifno> for u32 {
    fn from(ifno: Ifno) -> Self {
        u32::from(ifno.0)
    }
}

/// SuperSpeed endpoint companion of an endpoint without one.
const NO_SS_COMPANION: fusb_descriptor::UsbSsEpCompDescriptor =
    fusb_descriptor::UsbSsEpCompDescriptor {
        b_length: 0,
        b_descriptor_type: 0,
        b_max_burst: 0,
        bm_attributes: 0,
        w_bytes_per_interval: 0,
    };

#[derive(Clone, Debug, PartialEq)]
struct ParsedEndpoint {
    descriptor: fusb_descriptor::UsbEndpointDescriptor,
    ss_companion: fusb_descriptor::UsbSsEpCompDescriptor,
}

impl ParsedEndpoint {
    fn address(&self) -> u8 {
        self.descriptor.b_endpoint_address
    }

    fn transfer_type(&self) -> u32 {
        u32::from(self.descriptor.bm_attributes) & USB_ENDPOINT_XFERTYPE_MASK
    }

    fn max_packet_size(&self) -> u32 {
        u32::from(self.descriptor.w_max_packet_size) & USB_ENDPOINT_MAXP_MASK
    }
}

#[derive(Clone, Debug)]
struct ParsedInterfaceAlt {
    interface_number: u8,
    alt_setting: u8,
    interface_class: u8,
    endpoints: Vec<ParsedEndpoint>,
}

#[derive(Clone, Debug)]
struct ParsedConfig {
    config_value: u8,
    interfaces: Vec<ParsedInterfaceAlt>,
}

/// Returns an iterator over the descriptors in `data`.
///
/// Per USB 2.0 Section 9.5, every USB descriptor begins with a 2-byte `[bLength, bDescriptorType]`
/// header followed by `bLength - 2` descriptor-specific bytes. Iteration stops at the first
/// descriptor that is truncated or has a `bLength` smaller than the 2-byte header.
fn iter_descriptors(data: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut rest = data;
    std::iter::from_fn(move || {
        let length = usize::from(*rest.first()?);
        if length < 2 || length > rest.len() {
            return None;
        }
        let (descriptor, tail) = rest.split_at(length);
        rest = tail;
        Some(descriptor)
    })
}

fn parse_endpoint_descriptor(desc: &[u8]) -> Option<ParsedEndpoint> {
    if desc.len() < USB_DT_ENDPOINT_SIZE as usize {
        return None;
    }
    let descriptor = fusb_descriptor::UsbEndpointDescriptor {
        b_length: desc[0],
        b_descriptor_type: desc[1],
        b_endpoint_address: desc[ENDPOINT_DESC_ADDRESS],
        bm_attributes: desc[ENDPOINT_DESC_ATTRIBUTES],
        w_max_packet_size: u16::from_le_bytes([
            desc[ENDPOINT_DESC_MAX_PACKET_SIZE],
            desc[ENDPOINT_DESC_MAX_PACKET_SIZE + 1],
        ]),
        b_interval: desc[ENDPOINT_DESC_INTERVAL],
    };
    Some(ParsedEndpoint { descriptor, ss_companion: NO_SS_COMPANION })
}

fn parse_ss_companion_descriptor(desc: &[u8]) -> Option<fusb_descriptor::UsbSsEpCompDescriptor> {
    let (comp, _) = usb_ss_ep_comp_descriptor::read_from_prefix(desc).ok()?;
    Some(fusb_descriptor::UsbSsEpCompDescriptor {
        b_length: comp.bLength,
        b_descriptor_type: comp.bDescriptorType,
        b_max_burst: comp.bMaxBurst,
        bm_attributes: comp.bmAttributes,
        w_bytes_per_interval: u16::from_le(comp.wBytesPerInterval),
    })
}

/// Parses the configurations, interfaces, and endpoints of the configuration descriptors in
/// `data`.
///
/// Per USB 2.0 Section 9.4.3 (`GET_DESCRIPTOR` for `CONFIGURATION`) and USB 3.2 Section 9.6.7, a
/// configuration descriptor blob is a flat byte stream of concatenated `[bLength, bDescriptorType,
/// ...]` descriptors ordered hierarchically:
/// - Configuration descriptor (`USB_DT_CONFIG`)
///   - Interface alternate setting descriptors (`USB_DT_INTERFACE`)
///     - Endpoint descriptors (`USB_DT_ENDPOINT`)
///       - Optional class-specific endpoint descriptors
///       - Optional SuperSpeed Endpoint Companion descriptor (`USB_DT_SS_ENDPOINT_COMP`)
///
/// `data` concatenates the blobs of all configurations of the device. Descriptors that are too
/// short are ignored, as are interfaces before the first configuration and endpoints before the
/// first interface.
fn parse_config_descriptors(data: &[u8]) -> Vec<ParsedConfig> {
    let mut configs: Vec<ParsedConfig> = Vec::new();
    // Whether the last descriptor was an endpoint that may be followed by its SuperSpeed
    // endpoint companion descriptor.
    let mut in_endpoint_scope = false;

    for desc in iter_descriptors(data) {
        let descriptor_type = u32::from(desc[1]);
        let mut is_endpoint = false;
        match descriptor_type {
            USB_DT_CONFIG => {
                if let Ok((config, _)) = usb_config_descriptor::read_from_prefix(desc) {
                    configs.push(ParsedConfig {
                        config_value: config.bConfigurationValue,
                        interfaces: Vec::new(),
                    });
                }
            }
            USB_DT_INTERFACE => {
                let interface = usb_interface_descriptor::read_from_prefix(desc).ok();
                if let (Some((interface, _)), Some(config)) = (interface, configs.last_mut()) {
                    config.interfaces.push(ParsedInterfaceAlt {
                        interface_number: interface.bInterfaceNumber,
                        alt_setting: interface.bAlternateSetting,
                        interface_class: interface.bInterfaceClass,
                        endpoints: Vec::new(),
                    });
                }
            }
            USB_DT_ENDPOINT => {
                let interface = configs.last_mut().and_then(|c| c.interfaces.last_mut());
                if let (Some(endpoint), Some(interface)) =
                    (parse_endpoint_descriptor(desc), interface)
                {
                    interface.endpoints.push(endpoint);
                    is_endpoint = true;
                }
            }
            USB_DT_SS_ENDPOINT_COMP if in_endpoint_scope => {
                let endpoint = configs
                    .last_mut()
                    .and_then(|c| c.interfaces.last_mut())
                    .and_then(|i| i.endpoints.last_mut());
                if let (Some(companion), Some(endpoint)) =
                    (parse_ss_companion_descriptor(desc), endpoint)
                {
                    endpoint.ss_companion = companion;
                }
            }
            USB_DT_INTERFACE_ASSOCIATION => {}
            // Class-specific descriptors may follow an endpoint before its companion.
            _ => is_endpoint = in_endpoint_scope,
        }
        in_endpoint_scope = is_endpoint;
    }
    configs
}

fn zx_status_to_errno(status: i32) -> Errno {
    match status {
        zx::sys::ZX_ERR_IO_REFUSED => errno!(EPIPE),
        zx::sys::ZX_ERR_TIMED_OUT => errno!(ETIMEDOUT),
        zx::sys::ZX_ERR_NOT_FOUND => errno!(ENOENT),
        zx::sys::ZX_ERR_ALREADY_BOUND => errno!(EBUSY),
        zx::sys::ZX_ERR_IO_NOT_PRESENT | zx::sys::ZX_ERR_PEER_CLOSED => errno!(ENODEV),
        zx::sys::ZX_ERR_INVALID_ARGS => errno!(EINVAL),
        zx::sys::ZX_ERR_BAD_STATE => errno!(EHOSTUNREACH),
        zx::sys::ZX_ERR_CANCELED => errno!(ECONNRESET),
        zx::sys::ZX_ERR_NO_MEMORY => errno!(ENOMEM),
        _ => errno!(EIO),
    }
}

/// Maps a FIDL transport error to an errno: `ENODEV` if the channel is closed (the device is
/// gone), `EIO` otherwise.
fn fidl_error_to_errno(error: &fidl::Error) -> Errno {
    if error.is_closed() { errno!(ENODEV) } else { errno!(EIO) }
}

/// Maps the two-layer result of a FIDL method with an `error zx.Status` clause to an errno.
fn fidl_result_to_errno<T>(result: Result<Result<T, i32>, fidl::Error>) -> Result<T, Errno> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(status)) => Err(zx_status_to_errno(status)),
        Err(e) => Err(fidl_error_to_errno(&e)),
    }
}

/// Converts an optional timeout (`None` meaning no timeout) into an absolute deadline.
fn deadline_after(timeout: Option<zx::MonotonicDuration>) -> zx::MonotonicInstant {
    timeout.map_or(zx::MonotonicInstant::INFINITE, zx::MonotonicInstant::after)
}

/// Converts a usbfs timeout in milliseconds into an optional timeout. A timeout of 0 means no
/// timeout.
fn usbfs_timeout(timeout_ms: u32) -> Option<zx::MonotonicDuration> {
    (timeout_ms != 0).then(|| zx::MonotonicDuration::from_millis(i64::from(timeout_ms)))
}

/// Blocks the current task until any of the signals in `waits` is asserted on its handle, or
/// `deadline` passes.
///
/// The wait is killable, like Linux usbfs transfers: only unblockable signals (SIGKILL, SIGSTOP)
/// interrupt it, in which case `EINTR` is returned. Other signals stay pending and are delivered
/// when the syscall returns. Returns `ETIMEDOUT` when the deadline passes.
fn wait_for_signals(
    current_task: &CurrentTask,
    waits: &[(&dyn zx::AsHandleRef, zx::Signals)],
    deadline: zx::MonotonicInstant,
) -> Result<(), Errno> {
    // Block all the maskable signals for the duration of the wait, unless a temporary signal mask
    // is already in place.
    //
    // TODO(https://fxbug.dev/573607334): On Linux, an unblocked signal whose default action
    // terminates the process (e.g. SIGTERM with SIG_DFL, but not core-dumping signals, and not
    // when ptraced) is promoted to SIGKILL on delivery, which ends a killable wait. This mask
    // prevents that for single-threaded processes waiting without timeout (multi-threaded ones
    // still exit through another thread). Fixing it requires a killable wait primitive in the
    // Starnix core, since a core promotion honoring the current mask would not see through this
    // temporary one.
    let mask_signals = !current_task.flags().contains(TaskFlags::TEMPORARY_SIGNAL_MASK);
    if mask_signals {
        current_task.write().set_temporary_signal_mask(!SigSet::default());
    }
    let result = wait_for_signals_unmasked(current_task, waits, deadline);
    if mask_signals {
        // Signals received during the wait are delivered with the original mask.
        current_task.write().restore_signal_mask();
    }
    result
}

fn wait_for_signals_unmasked(
    current_task: &CurrentTask,
    waits: &[(&dyn zx::AsHandleRef, zx::Signals)],
    deadline: zx::MonotonicInstant,
) -> Result<(), Errno> {
    loop {
        // `PortWaiter::wait_until` automatically invokes `current_task.trigger_delayed_releaser()`
        // unless `WaiterOptions::UNSAFE_CALLSTACK` is set. Callers of `wait_for_signals` hold
        // `UsbfsEndpointTransferLock`, and flushing a closed `UsbDeviceFile` queued in the
        // thread-local delayed releaser runs `UsbDeviceFile::drop`, which acquires
        // `UsbfsDeviceStateLock` (ordered before `UsbfsEndpointTransferLock`).
        let waiter = Waiter::with_options(WaiterOptions::UNSAFE_CALLSTACK);
        let mut cancelers = Vec::with_capacity(waits.len());
        for &(handle, signals) in waits {
            let handler = SignalHandler {
                inner: SignalHandlerInner::None,
                event_handler: EventHandler::None,
                err_code: None,
            };
            cancelers.push(
                waiter.wake_on_zircon_signals(handle, signals, handler).map_err(|_| errno!(EIO))?,
            );
        }
        // The wait is edge-triggered, so check for signals asserted before it was registered.
        let asserted = waits.iter().any(|&(handle, signals)| {
            handle
                .as_handle_ref()
                .wait_one(signals, zx::MonotonicInstant::INFINITE_PAST)
                .to_result()
                .is_ok()
        });
        let result = if asserted { Ok(()) } else { waiter.wait_until(current_task, deadline) };
        for canceler in cancelers {
            canceler.cancel();
        }
        match result {
            // Spurious wakeup without a pending signal.
            Err(e)
                if e.code == EINTR
                    && !current_task.is_interrupt_pending()
                    && !current_task.is_exitted() =>
            {
                continue;
            }
            result => return result,
        }
    }
}

/// Kind of transfer issued by `USBDEVFS_BULK` on an endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransferType {
    Bulk,
    Interrupt,
}

impl TransferType {
    fn request_info(self) -> fusb_request::RequestInfo {
        match self {
            Self::Bulk => fusb_request::RequestInfo::Bulk(fusb_request::BulkRequestInfo::default()),
            Self::Interrupt => {
                fusb_request::RequestInfo::Interrupt(fusb_request::InterruptRequestInfo::default())
            }
        }
    }
}

/// How `USBDEVFS_BULK` transfers are issued on an endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct EndpointTransferParams {
    transfer_type: TransferType,
    /// Size of the requests a transfer is split into: the largest multiple of the endpoint's
    /// `wMaxPacketSize` that fits in the endpoint VMO, so that only the last request of a
    /// transfer can end with a short packet.
    chunk_size: usize,
}

impl EndpointTransferParams {
    /// Returns the parameters of `endpoint`, or `EINVAL` if the endpoint is neither a bulk nor
    /// an interrupt endpoint, or has a zero `wMaxPacketSize`.
    fn for_endpoint(endpoint: &ParsedEndpoint) -> Result<Self, Errno> {
        let max_packet_size = endpoint.max_packet_size() as usize;
        if max_packet_size == 0 {
            return error!(EINVAL);
        }
        let transfer_type = match endpoint.transfer_type() {
            USB_ENDPOINT_XFER_BULK => TransferType::Bulk,
            USB_ENDPOINT_XFER_INT => TransferType::Interrupt,
            _ => return error!(EINVAL),
        };
        let vmo_size = ENDPOINT_VMO_SIZE as usize;
        Ok(Self { transfer_type, chunk_size: vmo_size - vmo_size % max_packet_size })
    }
}

/// Returns the setup packet of a `CLEAR_FEATURE(ENDPOINT_HALT)` request for endpoint `ep_addr`.
fn clear_halt_setup(ep_addr: u8) -> fusb_descriptor::UsbSetup {
    fusb_descriptor::UsbSetup {
        bm_request_type: (USB_DIR_OUT | USB_TYPE_STANDARD | USB_RECIP_ENDPOINT) as u8,
        b_request: USB_REQ_CLEAR_FEATURE as u8,
        w_value: USB_ENDPOINT_HALT as u16,
        w_index: u16::from(ep_addr),
        w_length: 0,
    }
}

/// Returns the ID of the VMO used by the request of `completion`, if known.
fn completion_vmo_id(completion: &fusb_endpoint::Completion) -> Option<u64> {
    match completion.request.as_ref()?.data.as_ref()?.first()?.buffer {
        Some(fusb_request::Buffer::VmoId(id)) => Some(id),
        _ => None,
    }
}

/// State of an endpoint protected by its transfer lock.
struct EndpointTransferState {
    /// VMO used by the requests, at offset 0. At most one request is in flight at a time.
    vmo: zx::Vmo,
    vmo_id: u64,
    /// Number of requests queued with `vmo_id` whose completion has not been received yet.
    in_flight_count: usize,
    /// Number of requests whose completion has not been received yet, for each retired VMO.
    ///
    /// When the completion of a cancelled request does not arrive in time, its VMO is retired
    /// and replaced by a new one, since the request may still complete and write to it later.
    /// The completions of the requests of retired VMOs are discarded, and a retired VMO is
    /// unregistered once all of them have been received.
    retired_vmos: HashMap<u64, usize>,
    next_vmo_id: u64,
}

/// Lifecycle and reset state of an endpoint, protected by `UsbEndpointClient::state`.
///
/// Locking this state around both request submission (`queue_requests`) and endpoint disabling or
/// reset (`cancel_all`) ensures a transfer cannot queue a new request onto an endpoint after
/// `cancel_all` has already run for that disable or reset.
struct EndpointClientState {
    /// Whether the endpoint is enabled and new requests may be queued. Cleared before the
    /// in-flight requests are cancelled when the endpoint is disabled.
    usable: bool,
    /// Whether the driver closed the channel. The driver disables the endpoint before closing
    /// the channel, so a new connection can be made right away.
    closed: bool,
    /// Generation count incremented on each reset or cancellation of the endpoint.
    reset_count: u64,
}

/// Connection to an endpoint of the device.
///
/// The channel stays open for the lifetime of the device: closing it makes the host controller
/// cancel the requests and disable the endpoint asynchronously, which would race with a later
/// enable of the endpoint. Only the driver closes it, after disabling the endpoint.
pub(crate) struct UsbEndpointClient {
    proxy: fusb_endpoint::EndpointSynchronousProxy,
    /// Serializes the transfers on this endpoint. Held while waiting for completions, so it is
    /// ordered before `UninterruptibleLock`.
    transfer: LockDepMutex<EndpointTransferState, UsbfsEndpointTransferLock>,
    /// Protects the endpoint's enabled/closed status and reset generation across request
    /// submission and cancellation. Ordered after `UninterruptibleLock`, `UsbfsDeviceStateLock`,
    /// `UsbfsEndpointTransferLock`, and `UsbfsEndpointsLock`.
    state: LockDepMutex<EndpointClientState, UsbfsEndpointStateLock>,
    /// Signaled with `USER_0` to make a transfer waiting for a completion check whether it
    /// should stop, when the endpoint is disabled or reset.
    wake: zx::Event,
    /// Upper bound on the time spent draining cancelled requests.
    cancel_drain_timeout: zx::MonotonicDuration,
}

impl UsbEndpointClient {
    /// Connects to endpoint `ep_addr` of the device behind `proxy` and registers its VMO.
    fn connect(
        proxy: &fhardware_usb::UsbSynchronousProxy,
        ep_addr: u8,
        cancel_drain_timeout: zx::MonotonicDuration,
    ) -> Result<Arc<Self>, Errno> {
        let (ep_client_end, ep_server_end) = create_endpoints::<fusb_endpoint::EndpointMarker>();
        fidl_result_to_errno(proxy.connect_to_endpoint(
            ep_addr,
            ep_server_end,
            zx::MonotonicInstant::INFINITE,
        ))?;
        let ep_proxy = fusb_endpoint::EndpointSynchronousProxy::new(ep_client_end.into_channel());
        let vmo_id = 1;
        let vmo = Self::register_vmo(&ep_proxy, vmo_id)?;
        Ok(Arc::new(Self {
            proxy: ep_proxy,
            transfer: LockDepMutex::new(EndpointTransferState {
                vmo,
                vmo_id,
                in_flight_count: 0,
                retired_vmos: HashMap::new(),
                next_vmo_id: vmo_id + 1,
            }),
            state: LockDepMutex::new(EndpointClientState {
                usable: false,
                closed: false,
                reset_count: 0,
            }),
            wake: zx::Event::create(),
            cancel_drain_timeout,
        }))
    }

    fn register_vmo(
        proxy: &fusb_endpoint::EndpointSynchronousProxy,
        vmo_id: u64,
    ) -> Result<zx::Vmo, Errno> {
        let vmo_infos = [fusb_endpoint::VmoInfo {
            id: Some(vmo_id),
            size: Some(ENDPOINT_VMO_SIZE),
            ..Default::default()
        }];
        let mut vmos = proxy
            .register_vmos(&vmo_infos, zx::MonotonicInstant::INFINITE)
            .map_err(|e| fidl_error_to_errno(&e))?;
        vmos.pop().and_then(|h| h.vmo).ok_or_else(|| errno!(EIO))
    }

    fn is_closed(&self) -> bool {
        self.state.lock().closed
    }

    fn reset_count(&self) -> u64 {
        self.state.lock().reset_count
    }

    fn enable(&self) {
        self.state.lock().usable = true;
    }

    /// Makes a transfer waiting for a completion on this endpoint check whether it should stop.
    fn wake_transfer(&self) {
        let _ = self.wake.signal(zx::Signals::NONE, zx::Signals::USER_0);
    }

    /// Marks the endpoint as disabled and cancels its in-flight requests under the state lock, so
    /// no concurrent transfer can queue a request after `cancel_all` runs.
    fn disable(&self) {
        let mut state = self.state.lock();
        state.usable = false;
        state.reset_count += 1;
        self.wake_transfer();
        if let Err(e) =
            self.proxy.cancel_all(zx::MonotonicInstant::after(self.cancel_drain_timeout))
        {
            if e.is_closed() {
                state.closed = true;
            }
        }
    }

    /// Cancels the in-flight requests under the state lock. The transfer that queued them drains
    /// their completions.
    fn cancel_all(&self) {
        let mut state = self.state.lock();
        state.reset_count += 1;
        self.wake_transfer();
        if let Err(e) =
            self.proxy.cancel_all(zx::MonotonicInstant::after(self.cancel_drain_timeout))
        {
            if e.is_closed() {
                state.usable = false;
                state.closed = true;
            }
        }
    }

    /// Returns the error of a transfer stopped before or while waiting, if any: `ENOENT` if
    /// `cancel_token` is set, like a killed URB, or `ESHUTDOWN` if usbfs disabled or reset the
    /// endpoint, like URBs flushed by Linux `usb_hcd_flush_endpoint`.
    ///
    /// When `cancel_token` is provided, it is loaded with `Acquire` ordering so that any state
    /// published before setting the cancellation flag is visible to the transfer.
    fn cancellation_error_locked(
        state: &EndpointClientState,
        cancel_token: Option<&AtomicBool>,
        expected_reset: u64,
    ) -> Option<Errno> {
        if cancel_token.is_some_and(|t| t.load(Ordering::Acquire)) {
            Some(errno!(ENOENT))
        } else if !state.usable || state.reset_count != expected_reset {
            Some(errno!(ESHUTDOWN))
        } else {
            None
        }
    }

    fn cancellation_error(
        &self,
        cancel_token: Option<&AtomicBool>,
        expected_reset: u64,
    ) -> Option<Errno> {
        let state = self.state.lock();
        Self::cancellation_error_locked(&state, cancel_token, expected_reset)
    }

    fn mark_closed(&self) {
        let mut state = self.state.lock();
        state.usable = false;
        state.closed = true;
    }

    /// Maps a channel error to an errno, recording whether the driver closed the channel.
    fn channel_error(&self, error: &fidl::Error) -> Errno {
        if error.is_closed() {
            self.mark_closed();
        }
        fidl_error_to_errno(error)
    }

    /// Accounts for `completion`, returning it if it is the completion of the request in flight
    /// on the current VMO.
    fn account_completion(
        &self,
        transfer: &mut EndpointTransferState,
        completion: fusb_endpoint::Completion,
    ) -> Option<fusb_endpoint::Completion> {
        match completion_vmo_id(&completion) {
            Some(id) if id != transfer.vmo_id => {
                if let Some(count) = transfer.retired_vmos.get_mut(&id) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        transfer.retired_vmos.remove(&id);
                        let _ = self.proxy.unregister_vmos(&[id], zx::MonotonicInstant::INFINITE);
                    }
                }
                None
            }
            // Completions received while no request is in flight are stale.
            _ if transfer.in_flight_count == 0 => None,
            _ => {
                transfer.in_flight_count -= 1;
                Some(completion)
            }
        }
    }

    /// Accounts for the completions of the event `event`, returning the completion of the
    /// request in flight on the current VMO, if any.
    fn account_event(
        &self,
        transfer: &mut EndpointTransferState,
        event: fusb_endpoint::EndpointEvent,
    ) -> Option<fusb_endpoint::Completion> {
        let fusb_endpoint::EndpointEvent::OnCompletion { completion } = event;
        let mut current = None;
        for completion in completion {
            if let Some(completion) = self.account_completion(transfer, completion) {
                current = Some(completion);
            }
        }
        current
    }

    /// Accounts for the completions already received, without blocking.
    fn process_pending_events(&self, transfer: &mut EndpointTransferState) -> Result<(), Errno> {
        loop {
            match self.proxy.wait_for_event(zx::MonotonicInstant::INFINITE_PAST) {
                Ok(event) => {
                    self.account_event(transfer, event);
                }
                Err(fidl::Error::ClientEvent(zx::Status::TIMED_OUT)) => return Ok(()),
                Err(fidl::Error::UnexpectedSyncResponse) => continue,
                Err(e) => return Err(self.channel_error(&e)),
            }
        }
    }

    /// Replaces the VMO by a new one if requests are still in flight on it.
    fn retire_vmo_if_busy(&self, transfer: &mut EndpointTransferState) -> Result<(), Errno> {
        if transfer.in_flight_count == 0 {
            return Ok(());
        }
        if transfer.retired_vmos.len() >= MAX_RETIRED_VMOS {
            return error!(EIO);
        }
        let vmo_id = transfer.next_vmo_id;
        let vmo = Self::register_vmo(&self.proxy, vmo_id).inspect_err(|e| {
            if e.code == ENODEV {
                self.mark_closed();
            }
        })?;
        transfer.next_vmo_id += 1;
        transfer.retired_vmos.insert(transfer.vmo_id, transfer.in_flight_count);
        transfer.vmo = vmo;
        transfer.vmo_id = vmo_id;
        transfer.in_flight_count = 0;
        Ok(())
    }

    /// Checks that a transfer can start, and makes sure that no request is in flight on the
    /// VMO.
    fn begin_transfer(
        &self,
        transfer: &mut EndpointTransferState,
        cancel_token: Option<&AtomicBool>,
        expected_reset: u64,
    ) -> Result<(), Errno> {
        {
            let state = self.state.lock();
            if state.closed {
                return error!(ENODEV);
            }
            if let Some(error) =
                Self::cancellation_error_locked(&state, cancel_token, expected_reset)
            {
                return Err(error);
            }
        }
        self.process_pending_events(transfer)?;
        self.retire_vmo_if_busy(transfer)
    }

    /// Waits for the completion of the request in flight on the current VMO.
    ///
    /// Returns the error of `cancellation_error` if the transfer is stopped while waiting, in
    /// which case the request is still in flight.
    fn wait_for_completion(
        &self,
        current_task: &CurrentTask,
        transfer: &mut EndpointTransferState,
        deadline: zx::MonotonicInstant,
        cancel_token: Option<&AtomicBool>,
        expected_reset: u64,
    ) -> Result<fusb_endpoint::Completion, Errno> {
        loop {
            match self.proxy.wait_for_event(zx::MonotonicInstant::INFINITE_PAST) {
                Ok(event) => {
                    if let Some(completion) = self.account_event(transfer, event) {
                        return Ok(completion);
                    }
                    continue;
                }
                Err(fidl::Error::ClientEvent(zx::Status::TIMED_OUT)) => {}
                Err(fidl::Error::UnexpectedSyncResponse) => continue,
                Err(e) => return Err(self.channel_error(&e)),
            }
            // Clear the wake signal before checking the conditions it reports, so that a later
            // change is not missed.
            let _ = self.wake.signal(zx::Signals::USER_0, zx::Signals::NONE);
            if let Some(error) = self.cancellation_error(cancel_token, expected_reset) {
                return Err(error);
            }
            wait_for_signals(
                current_task,
                &[
                    (
                        self.proxy.as_channel(),
                        zx::Signals::CHANNEL_READABLE | zx::Signals::CHANNEL_PEER_CLOSED,
                    ),
                    (&self.wake, zx::Signals::USER_0),
                ],
                deadline,
            )?;
        }
    }

    /// Cancels the request in flight and waits for its completion, which is returned if it
    /// arrives in time. Otherwise, the VMO is retired before the next transfer.
    fn cancel_and_drain(
        &self,
        transfer: &mut EndpointTransferState,
    ) -> Option<fusb_endpoint::Completion> {
        let deadline = zx::MonotonicInstant::after(self.cancel_drain_timeout);
        match self.proxy.cancel_all(deadline) {
            Ok(Ok(())) => {}
            Ok(Err(_)) => return None,
            Err(e) => {
                let _ = self.channel_error(&e);
                return None;
            }
        }
        let mut current = None;
        while transfer.in_flight_count > 0 {
            match self.proxy.wait_for_event(deadline) {
                Ok(event) => {
                    if let Some(completion) = self.account_event(transfer, event) {
                        current = Some(completion);
                    }
                }
                Err(fidl::Error::UnexpectedSyncResponse) => continue,
                Err(e) => {
                    let _ = self.channel_error(&e);
                    break;
                }
            }
        }
        current
    }

    /// Queues one request of `len` bytes at offset 0 of the endpoint VMO and waits for its
    /// completion. Returns the number of bytes transferred, or `EOVERFLOW` if the completion
    /// reports more than `len` bytes.
    ///
    /// On timeout, interruption, or cancellation, the request is cancelled. If it nevertheless
    /// completed successfully, its result is returned. Otherwise, `ETIMEDOUT`, `EINTR`, or the
    /// error of `cancellation_error` is returned.
    fn transfer_chunk(
        &self,
        current_task: &CurrentTask,
        transfer: &mut EndpointTransferState,
        transfer_type: TransferType,
        len: usize,
        deadline: zx::MonotonicInstant,
        cancel_token: Option<&AtomicBool>,
        expected_reset: u64,
    ) -> Result<usize, Errno> {
        let request = fusb_request::Request {
            data: Some(vec![fusb_request::BufferRegion {
                buffer: Some(fusb_request::Buffer::VmoId(transfer.vmo_id)),
                offset: Some(0),
                size: Some(len as u64),
                ..Default::default()
            }]),
            defer_completion: Some(false),
            information: Some(transfer_type.request_info()),
            ..Default::default()
        };
        // Hold the endpoint state lock while checking for cancellation and queueing the request so
        // that `disable` or `cancel_all` cannot run before `queue_requests` and leave the newly
        // queued request uncancelled on a disabled or reset endpoint.
        {
            let state = self.state.lock();
            if let Some(error) =
                Self::cancellation_error_locked(&state, cancel_token, expected_reset)
            {
                return Err(error);
            }
            if let Err(e) = self.proxy.queue_requests(vec![request]) {
                drop(state);
                return Err(self.channel_error(&e));
            }
        }
        transfer.in_flight_count += 1;
        let completion = match self.wait_for_completion(
            current_task,
            transfer,
            deadline,
            cancel_token,
            expected_reset,
        ) {
            Ok(completion) => completion,
            Err(e)
                if e.code == ETIMEDOUT
                    || e.code == EINTR
                    || e.code == ENOENT
                    || e.code == ESHUTDOWN =>
            {
                match self.cancel_and_drain(transfer) {
                    Some(completion) if completion.status == Some(zx::sys::ZX_OK) => completion,
                    _ => return Err(e),
                }
            }
            Err(e) => return Err(e),
        };
        let status = completion.status.unwrap_or(zx::sys::ZX_ERR_IO);
        if status != zx::sys::ZX_OK {
            // The cancelled completion of a request may be received before the transfer notices
            // why it was cancelled.
            if status == zx::sys::ZX_ERR_CANCELED {
                if let Some(error) = self.cancellation_error(cancel_token, expected_reset) {
                    return Err(error);
                }
            }
            return Err(zx_status_to_errno(status));
        }
        let transferred = completion.transfer_size.unwrap_or(0);
        // Like a babble error on Linux: the device sent more data than requested.
        usize::try_from(transferred).ok().filter(|&t| t <= len).ok_or_else(|| errno!(EOVERFLOW))
    }

    /// Sends `data`, in requests of `params.chunk_size` bytes.
    ///
    /// Returns the number of bytes sent, which is short if the device accepted less than a full
    /// request. Any error, including a timeout, is returned instead of the partial count.
    fn bulk_out(
        &self,
        current_task: &CurrentTask,
        params: EndpointTransferParams,
        data: &[u8],
        deadline: zx::MonotonicInstant,
        cancel_token: Option<&AtomicBool>,
        expected_reset: u64,
    ) -> Result<usize, Errno> {
        // All transfers on this endpoint are serialized by `self.transfer`, and `begin_transfer`
        // / `cancel_and_drain` ensure no earlier request is still using `transfer.vmo` (replacing
        // it with a fresh VMO if a cancelled request did not drain in time).
        let mut transfer = self.transfer.lock();
        self.begin_transfer(&mut transfer, cancel_token, expected_reset)?;

        if data.is_empty() {
            self.transfer_chunk(
                current_task,
                &mut transfer,
                params.transfer_type,
                0,
                deadline,
                cancel_token,
                expected_reset,
            )?;
            return Ok(0);
        }

        let mut offset = 0;
        while offset < data.len() {
            if let Some(error) = self.cancellation_error(cancel_token, expected_reset) {
                return Err(error);
            }
            let chunk_len = min(data.len() - offset, params.chunk_size);
            transfer.vmo.write(&data[offset..offset + chunk_len], 0).map_err(|_| errno!(EIO))?;
            let _ = transfer.vmo.op_range(zx::VmoOp::CACHE_CLEAN, 0, chunk_len as u64);
            let actual = self.transfer_chunk(
                current_task,
                &mut transfer,
                params.transfer_type,
                chunk_len,
                deadline,
                cancel_token,
                expected_reset,
            )?;
            offset += actual;
            if actual < chunk_len {
                break;
            }
        }
        Ok(offset)
    }

    /// Receives up to `max_len` bytes, in requests of at most `params.chunk_size` bytes.
    ///
    /// The transfer ends early on a short request. Any error, including a timeout, is returned
    /// and the data received so far is discarded.
    fn bulk_in(
        &self,
        current_task: &CurrentTask,
        params: EndpointTransferParams,
        max_len: usize,
        deadline: zx::MonotonicInstant,
        cancel_token: Option<&AtomicBool>,
        expected_reset: u64,
    ) -> Result<Vec<u8>, Errno> {
        // All transfers on this endpoint are serialized by `self.transfer`, and `begin_transfer`
        // / `cancel_and_drain` ensure no earlier request is still using `transfer.vmo` (replacing
        // it with a fresh VMO if a cancelled request did not drain in time).
        let mut transfer = self.transfer.lock();
        self.begin_transfer(&mut transfer, cancel_token, expected_reset)?;

        if max_len == 0 {
            self.transfer_chunk(
                current_task,
                &mut transfer,
                params.transfer_type,
                0,
                deadline,
                cancel_token,
                expected_reset,
            )?;
            return Ok(Vec::new());
        }

        let mut result = Vec::new();
        while result.len() < max_len {
            if let Some(error) = self.cancellation_error(cancel_token, expected_reset) {
                return Err(error);
            }
            let requested = min(max_len - result.len(), params.chunk_size);
            let _ = transfer.vmo.op_range(zx::VmoOp::CACHE_CLEAN_INVALIDATE, 0, requested as u64);
            let actual = self.transfer_chunk(
                current_task,
                &mut transfer,
                params.transfer_type,
                requested,
                deadline,
                cancel_token,
                expected_reset,
            )?;
            if actual > 0 {
                let _ = transfer.vmo.op_range(zx::VmoOp::CACHE_CLEAN_INVALIDATE, 0, actual as u64);
                let start = result.len();
                result.resize(start + actual, 0);
                transfer.vmo.read(&mut result[start..], 0).map_err(|_| errno!(EIO))?;
            }
            if actual < requested {
                // Short packet: the device has no more data for this transfer.
                break;
            }
        }
        Ok(result)
    }
}

/// Serializes the synchronous control transfers of a device.
///
/// The `Usb` protocol cannot cancel a control transfer, so a transfer whose caller is killed
/// keeps running on its kernel thread until it completes, which may be never if the caller gave
/// no timeout. Later transfers wait for it rather than piling up blocked kernel threads.
struct ControlTransferSlot {
    /// Whether a control transfer is currently in flight. Protected by a mutex rather than an
    /// atomic so that clearing/signaling the `released` event and updating `busy` are serialized
    /// without lock-free ordering subtleties.
    busy: LockDepMutex<bool, UsbfsControlSlotLock>,
    /// `USER_0` is asserted when the slot is released.
    released: zx::Event,
}

impl ControlTransferSlot {
    fn new() -> Self {
        Self { busy: LockDepMutex::new(false), released: zx::Event::create() }
    }

    /// Takes the slot, waiting for the transfer holding it to complete. The wait is killable.
    fn acquire(&self, current_task: &CurrentTask) -> Result<(), Errno> {
        loop {
            {
                let mut busy = self.busy.lock();
                if !*busy {
                    *busy = true;
                    return Ok(());
                }
                // Clear the release signal while holding `busy` so a concurrent `release` cannot
                // signal `USER_0` between the check and the clear.
                let _ = self.released.signal(zx::Signals::USER_0, zx::Signals::NONE);
            }
            wait_for_signals(
                current_task,
                &[(&self.released, zx::Signals::USER_0)],
                zx::MonotonicInstant::INFINITE,
            )?;
        }
    }

    fn release(&self) {
        let mut busy = self.busy.lock();
        *busy = false;
        let _ = self.released.signal(zx::Signals::NONE, zx::Signals::USER_0);
    }
}

/// Configuration state of a device, shared by all its file descriptors.
pub(crate) struct DeviceState {
    /// `bConfigurationValue` of the active configuration (0 if unconfigured).
    pub(crate) active_config: u8,
    /// Active alternate setting of each interface (absent means 0).
    alt_settings: HashMap<Ifno, u8>,
    /// File descriptor (`UsbDeviceFile::id`) owning each claimed interface.
    claims: HashMap<Ifno, u64>,
    /// Endpoints enabled on the host controller, with the descriptors they were enabled with.
    enabled_endpoints: HashMap<u8, ParsedEndpoint>,
}

pub(crate) struct UsbDevice {
    pub(crate) bus_num: u16,
    pub(crate) dev_num: u16,
    pub(crate) speed: u32,
    pub(crate) device_descriptor: usb_device_descriptor,
    parsed_configs: Vec<ParsedConfig>,
    pub(crate) raw_descriptors: Vec<u8>,
    pub(crate) sync_proxy: Option<Arc<fhardware_usb::UsbSynchronousProxy>>,
    /// Serializes the ioctls that read or change the configuration, interface claims, and
    /// endpoint lifecycle. Held across synchronous FIDL calls to the USB driver, so it is ordered
    /// before `UninterruptibleLock`.
    pub(crate) state: LockDepMutex<DeviceState, UsbfsDeviceStateLock>,
    pub(crate) endpoints: LockDepMutex<HashMap<u8, Arc<UsbEndpointClient>>, UsbfsEndpointsLock>,
    /// Set once the device has been removed from usbfs. Stored with `Release` ordering (while
    /// holding `endpoints` in `on_disconnect`) and loaded with `Acquire` ordering so that any
    /// thread observing `disconnected == false` under `endpoints` is guaranteed that
    /// `on_disconnect` has not yet drained the endpoint map, and any thread observing
    /// `disconnected == true` after a transfer error sees the disconnect state established by
    /// `on_disconnect`.
    disconnected: AtomicBool,
    /// Upper bound on the time spent draining cancelled requests before retiring an endpoint
    /// VMO.
    pub(crate) cancel_drain_timeout: zx::MonotonicDuration,
    control_slot: Arc<ControlTransferSlot>,
    /// Monotonically increasing counter assigning a unique ID to each `UsbDeviceFile`. Accessed
    /// with `Relaxed` ordering because the atomic `fetch_add` itself guarantees uniqueness and no
    /// other memory state is published through the ID.
    next_file_id: AtomicU64,
}

impl UsbDevice {
    pub(crate) fn new(
        bus_num: u16,
        dev_num: u16,
        speed: u32,
        device_descriptor: usb_device_descriptor,
        config_descriptors: Vec<u8>,
    ) -> Self {
        let mut raw_descriptors = Vec::new();
        let desc_bytes = zerocopy::IntoBytes::as_bytes(&device_descriptor);
        raw_descriptors.extend_from_slice(desc_bytes);
        raw_descriptors.extend_from_slice(&config_descriptors);
        let parsed_configs = parse_config_descriptors(&config_descriptors);

        Self {
            bus_num,
            dev_num,
            speed,
            device_descriptor,
            parsed_configs,
            raw_descriptors,
            sync_proxy: None,
            state: LockDepMutex::new(DeviceState {
                active_config: 1,
                alt_settings: HashMap::new(),
                claims: HashMap::new(),
                enabled_endpoints: HashMap::new(),
            }),
            endpoints: LockDepMutex::new(HashMap::new()),
            disconnected: AtomicBool::new(false),
            cancel_drain_timeout: DEFAULT_CANCEL_DRAIN_TIMEOUT,
            control_slot: Arc::new(ControlTransferSlot::new()),
            next_file_id: AtomicU64::new(1),
        }
    }

    /// Computes the Linux USB device minor number per `usb_dev_minor`:
    /// `(bus - 1) * 128 + (dev_num - 1)`.
    pub(crate) fn minor(&self) -> u32 {
        debug_assert!(self.bus_num >= 1 && self.dev_num >= 1);
        (u32::from(self.bus_num).saturating_sub(1) * 128)
            + u32::from(self.dev_num).saturating_sub(1)
    }

    pub(crate) fn speed_string(&self) -> &'static str {
        match self.speed {
            USB_SPEED_LOW => "1.5\n",
            USB_SPEED_FULL => "12\n",
            USB_SPEED_HIGH => "480\n",
            USB_SPEED_WIRELESS => "480\n",
            USB_SPEED_SUPER => "5000\n",
            USB_SPEED_SUPER_PLUS => "10000\n",
            _ => "unknown\n",
        }
    }

    /// Returns the proxy to the USB device, or `ENODEV` if there is none.
    fn proxy(&self) -> Result<&Arc<fhardware_usb::UsbSynchronousProxy>, Errno> {
        self.sync_proxy.as_ref().ok_or_else(|| errno!(ENODEV))
    }

    /// Returns the active configuration, or `None` if the device is unconfigured.
    fn active_config(&self, state: &DeviceState) -> Option<&ParsedConfig> {
        self.parsed_configs.iter().find(|c| c.config_value == state.active_config)
    }

    fn has_interface(&self, state: &DeviceState, ifno: Ifno) -> bool {
        self.active_config(state)
            .is_some_and(|c| c.interfaces.iter().any(|i| i.interface_number == ifno.as_u8()))
    }

    /// Returns the alternate settings of interface `ifno` in the active configuration.
    fn interface_alts<'a>(
        &'a self,
        state: &DeviceState,
        ifno: Ifno,
    ) -> impl Iterator<Item = &'a ParsedInterfaceAlt> {
        self.active_config(state)
            .into_iter()
            .flat_map(|c| c.interfaces.iter())
            .filter(move |i| i.interface_number == ifno.as_u8())
    }

    fn find_interface_alt(
        &self,
        state: &DeviceState,
        interface_number: u8,
        alt: u8,
    ) -> Option<&ParsedInterfaceAlt> {
        self.active_config(state)?
            .interfaces
            .iter()
            .find(|i| i.interface_number == interface_number && i.alt_setting == alt)
    }

    /// Returns the addresses of the endpoints of interface `ifno` in all its alternate settings
    /// of the active configuration.
    fn interface_endpoint_addresses(&self, state: &DeviceState, ifno: Ifno) -> Vec<u8> {
        let mut addresses: Vec<u8> = self
            .interface_alts(state, ifno)
            .flat_map(|i| i.endpoints.iter().map(ParsedEndpoint::address))
            .collect();
        addresses.sort_unstable();
        addresses.dedup();
        addresses
    }

    /// Finds the interface number owning endpoint `ep` in any alternate setting of the active
    /// configuration.
    /// Returns `EINVAL` if bits outside `USB_DIR_IN | USB_ENDPOINT_NUMBER_MASK` are set, `ESRCH`
    /// if the device is unconfigured, and `ENOENT` if no interface has the endpoint.
    fn find_interface_for_endpoint(&self, state: &DeviceState, ep: u32) -> Result<Ifno, Errno> {
        if (ep & !(USB_DIR_IN | USB_ENDPOINT_NUMBER_MASK)) != 0 {
            return error!(EINVAL);
        }
        let config = self.active_config(state).ok_or_else(|| errno!(ESRCH))?;
        config
            .interfaces
            .iter()
            .find(|i| i.endpoints.iter().any(|e| u32::from(e.address()) == ep))
            .map_or_else(|| error!(ENOENT), |i| Ifno::try_from(u32::from(i.interface_number)))
    }

    /// Returns the descriptors of endpoint `ep_addr` in the active alternate settings, if any.
    fn active_endpoint(&self, state: &DeviceState, ep_addr: u8) -> Option<&ParsedEndpoint> {
        self.active_config(state)?
            .interfaces
            .iter()
            .filter(|i| {
                let ifno = Ifno(i.interface_number);
                i.alt_setting == state.alt_settings.get(&ifno).copied().unwrap_or(0)
            })
            .flat_map(|i| i.endpoints.iter())
            .find(|e| e.address() == ep_addr)
    }

    pub(crate) fn on_disconnect(&self) {
        let endpoints: Vec<_> = {
            let mut endpoints = self.endpoints.lock();
            self.disconnected.store(true, Ordering::Release);
            endpoints.drain().collect()
        };
        for (_ep_addr, client) in endpoints {
            client.disable();
        }
    }

    /// Returns a client for endpoint `endpoint` of the active alternate settings, enabling the
    /// endpoint on the host controller and connecting to it if needed.
    fn get_or_connect_endpoint(
        &self,
        state: &mut DeviceState,
        endpoint: &ParsedEndpoint,
    ) -> Result<Arc<UsbEndpointClient>, Errno> {
        let proxy = self.proxy()?;
        let ep_addr = endpoint.address();

        let existing = {
            let mut endpoints = self.endpoints.lock();
            if self.disconnected.load(Ordering::Acquire) {
                return error!(ENODEV);
            }
            match endpoints.get(&ep_addr).cloned() {
                Some(client) if client.is_closed() => {
                    // The driver closes the channel only after disabling the endpoint, so the
                    // endpoint can be enabled and connected again right away.
                    endpoints.remove(&ep_addr);
                    state.enabled_endpoints.remove(&ep_addr);
                    None
                }
                existing => existing,
            }
        };
        if let Some(client) = &existing {
            if state.enabled_endpoints.get(&ep_addr) == Some(endpoint) {
                return Ok(Arc::clone(client));
            }
        }
        // If the endpoint is still enabled with other descriptors, disable it first. The
        // lifecycle code disables endpoints whose interface changes, so this is a safety net.
        self.disable_endpoint(state, ep_addr)?;

        fidl_result_to_errno(proxy.enable_endpoint(
            &endpoint.descriptor,
            &endpoint.ss_companion,
            true,
            zx::MonotonicInstant::INFINITE,
        ))?;
        let client = match existing {
            Some(client) => client,
            None => match UsbEndpointClient::connect(proxy, ep_addr, self.cancel_drain_timeout) {
                Ok(client) => client,
                Err(e) => {
                    // Roll back the enable so a later attempt does not fail on an endpoint that
                    // is already enabled.
                    let _ = proxy.enable_endpoint(
                        &endpoint.descriptor,
                        &endpoint.ss_companion,
                        false,
                        zx::MonotonicInstant::INFINITE,
                    );
                    return Err(e);
                }
            },
        };
        {
            let mut endpoints = self.endpoints.lock();
            // `on_disconnect` may have run while `enable_endpoint` or `connect` was in flight
            // without the `endpoints` lock held. Re-check `disconnected` under `endpoints` before
            // publishing the enabled client so we never leave an active endpoint in `endpoints`
            // after disconnect.
            if self.disconnected.load(Ordering::Acquire) {
                drop(endpoints);
                client.disable();
                let _ = proxy.enable_endpoint(
                    &endpoint.descriptor,
                    &endpoint.ss_companion,
                    false,
                    zx::MonotonicInstant::INFINITE,
                );
                return error!(ENODEV);
            }
            client.enable();
            endpoints.insert(ep_addr, Arc::clone(&client));
        }
        state.enabled_endpoints.insert(ep_addr, endpoint.clone());
        Ok(client)
    }

    /// Disables endpoint `ep_addr` on the host controller if it is enabled, after cancelling its
    /// in-flight requests.
    ///
    /// This does not wait for the transfer that queued the requests: it is woken up and drains
    /// their completions itself, within the cancel drain timeout.
    fn disable_endpoint(&self, state: &mut DeviceState, ep_addr: u8) -> Result<(), Errno> {
        let Some(endpoint) = state.enabled_endpoints.remove(&ep_addr) else {
            return Ok(());
        };
        let client = self.endpoints.lock().get(&ep_addr).cloned();
        if let Some(client) = client {
            client.disable();
            if client.is_closed() {
                // The driver disabled the endpoint before closing the channel.
                self.endpoints.lock().remove(&ep_addr);
                return Ok(());
            }
        }
        fidl_result_to_errno(self.proxy()?.enable_endpoint(
            &endpoint.descriptor,
            &endpoint.ss_companion,
            false,
            zx::MonotonicInstant::INFINITE,
        ))
    }

    /// Disables all the enabled endpoints.
    fn disable_all_endpoints(&self, state: &mut DeviceState) -> Result<(), Errno> {
        let addresses: Vec<u8> = state.enabled_endpoints.keys().copied().collect();
        for ep_addr in addresses {
            self.disable_endpoint(state, ep_addr)?;
        }
        Ok(())
    }

    /// Cancels the in-flight requests of endpoint `ep_addr`.
    pub(crate) fn reset_endpoint_state(&self, ep_addr: u8) {
        let client = self.endpoints.lock().get(&ep_addr).cloned();
        if let Some(client) = client {
            client.cancel_all();
        }
    }

    /// Releases the claim on interface `ifno`, cancelling the in-flight requests of its
    /// endpoints. If a non-zero alternate setting was active, restores alternate setting 0.
    fn drop_claim(&self, state: &mut DeviceState, ifno: Ifno) {
        state.claims.remove(&ifno);
        let alt = state.alt_settings.remove(&ifno).unwrap_or(0);
        if alt != 0 && !self.disconnected.load(Ordering::Acquire) {
            for ep_addr in self.interface_endpoint_addresses(state, ifno) {
                let _ = self.disable_endpoint(state, ep_addr);
            }
            if let Ok(proxy) = self.proxy() {
                let _ = proxy.set_interface(ifno.as_u8(), 0, zx::MonotonicInstant::INFINITE);
            }
        } else {
            for ep_addr in self.interface_endpoint_addresses(state, ifno) {
                self.reset_endpoint_state(ep_addr);
            }
        }
    }

    /// Sends `data` to OUT endpoint `client`. See `UsbEndpointClient::bulk_out`.
    fn endpoint_bulk_out(
        &self,
        current_task: &CurrentTask,
        client: &UsbEndpointClient,
        params: EndpointTransferParams,
        data: &[u8],
        deadline: zx::MonotonicInstant,
        expected_reset: u64,
    ) -> Result<usize, Errno> {
        if self.disconnected.load(Ordering::Acquire) {
            return error!(ENODEV);
        }
        client
            .bulk_out(current_task, params, data, deadline, None, expected_reset)
            .map_err(|e| if self.disconnected.load(Ordering::Acquire) { errno!(ENODEV) } else { e })
    }

    /// Receives up to `max_len` bytes from IN endpoint `client`. See
    /// `UsbEndpointClient::bulk_in`.
    fn endpoint_bulk_in(
        &self,
        current_task: &CurrentTask,
        client: &UsbEndpointClient,
        params: EndpointTransferParams,
        max_len: usize,
        deadline: zx::MonotonicInstant,
        expected_reset: u64,
    ) -> Result<Vec<u8>, Errno> {
        if self.disconnected.load(Ordering::Acquire) {
            return error!(ENODEV);
        }
        client
            .bulk_in(current_task, params, max_len, deadline, None, expected_reset)
            .map_err(|e| if self.disconnected.load(Ordering::Acquire) { errno!(ENODEV) } else { e })
    }

    /// Issues a control transfer on endpoint 0 and returns the data read, if any.
    ///
    /// The transfer runs on a kernel thread so that the calling task can be killed while it
    /// waits. If it is, `EINTR` is returned and the transfer keeps running in the background
    /// until it completes, which never happens if `timeout` is `None` and the device does not
    /// answer. Transfers are serialized, so at most one such transfer runs per device.
    fn control_transfer(
        &self,
        current_task: &CurrentTask,
        setup: fusb_descriptor::UsbSetup,
        timeout: Option<zx::MonotonicDuration>,
        out_data: Vec<u8>,
    ) -> Result<Vec<u8>, Errno> {
        let proxy = Arc::clone(self.proxy()?);
        self.control_slot.acquire(current_task)?;
        // The Usb protocol interprets a timeout of 0 as its default timeout, so request an
        // infinite one explicitly.
        let timeout_ns = timeout.unwrap_or(zx::MonotonicDuration::INFINITE).into_nanos();
        let done = Arc::new(zx::Event::create());
        let (sender, receiver) = sync_channel(1);
        let done_for_transfer = Arc::clone(&done);
        let slot = Arc::clone(&self.control_slot);
        let closure = move |_: &CurrentTask| {
            let result = fidl_result_to_errno(proxy.control_transfer(
                &setup,
                timeout_ns,
                &out_data,
                zx::MonotonicInstant::INFINITE,
            ));
            slot.release();
            let _ = sender.send(result);
            let _ = done_for_transfer.signal(zx::Signals::NONE, zx::Signals::USER_0);
        };
        let request = SpawnRequestBuilder::new()
            .with_debug_name("usbfs-control-transfer")
            .with_sync_closure(closure)
            .build();
        current_task.kernel().kthreads.spawner().spawn_from_request(request);
        wait_for_signals(
            current_task,
            &[(done.as_ref(), zx::Signals::USER_0)],
            zx::MonotonicInstant::INFINITE,
        )?;
        receiver.recv().map_err(|_| errno!(EIO))?
    }
}

pub(crate) struct UsbDeviceFile {
    device: Arc<UsbDevice>,
    /// Identifies this file descriptor in `DeviceState::claims`.
    id: u64,
}

/// Reads the interface number argument of `USBDEVFS_CLAIMINTERFACE` and
/// `USBDEVFS_RELEASEINTERFACE`.
fn read_ifno(current_task: &CurrentTask, arg: SyscallArg) -> Result<Ifno, Errno> {
    let raw_ifno: u32 = current_task.read_object(UserRef::new(UserAddress::from(arg)))?;
    Ifno::try_from(raw_ifno)
}

impl UsbDeviceFile {
    pub(crate) fn new(device: Arc<UsbDevice>) -> Self {
        let id = device.next_file_id.fetch_add(1, Ordering::Relaxed);
        Self { device, id }
    }

    /// Claims interface `ifno` for this file descriptor:
    /// - Already claimed by this FD -> `Ok(())`
    /// - Interface does not exist in active configuration -> `ENOENT`
    /// - Claimed by another FD -> `EBUSY`
    fn claim_interface(&self, state: &mut DeviceState, ifno: Ifno) -> Result<(), Errno> {
        match state.claims.get(&ifno) {
            Some(&owner) if owner == self.id => return Ok(()),
            Some(_) => return error!(EBUSY),
            None => {}
        }
        if !self.device.has_interface(state, ifno) {
            return error!(ENOENT);
        }
        state.claims.insert(ifno, self.id);
        Ok(())
    }

    /// Releases interface `ifno` from this file descriptor:
    /// - Interface does not exist in active configuration -> `ENOENT`
    /// - Not claimed by this FD -> `EINVAL`
    fn release_interface(&self, state: &mut DeviceState, ifno: Ifno) -> Result<(), Errno> {
        if !self.device.has_interface(state, ifno) {
            return error!(ENOENT);
        }
        if state.claims.get(&ifno) != Some(&self.id) {
            return error!(EINVAL);
        }
        self.device.drop_claim(state, ifno);
        Ok(())
    }

    /// Ensures interface `ifnum` is claimed by this file descriptor, auto-claiming it if
    /// currently unclaimed. Returns `EHOSTUNREACH` if the device is unconfigured, then `EINVAL`
    /// if `ifnum` is out of range.
    fn check_interface(&self, state: &mut DeviceState, ifnum: u32) -> Result<(), Errno> {
        if self.device.active_config(state).is_none() {
            return error!(EHOSTUNREACH);
        }
        self.claim_interface(state, Ifno::try_from(ifnum)?)
    }

    /// Checks that this file descriptor may issue a control transfer to the recipient selected
    /// by `request_type` and `index`, auto-claiming the target interface if needed.
    fn check_control_recipient(
        &self,
        state: &mut DeviceState,
        request_type: u8,
        request: u8,
        index: u16,
    ) -> Result<(), Errno> {
        let request_type = u32::from(request_type);
        if (request_type & USB_TYPE_MASK) == USB_TYPE_VENDOR {
            return Ok(());
        }
        let [index_high, index_low] = index.to_be_bytes();
        // The printer class GET_DEVICE_ID request is always allowed: it is used to query
        // things like ink levels.
        if request_type == USB_PRINTER_GET_DEVICE_ID_REQUEST_TYPE
            && request == USB_PRINTER_GET_DEVICE_ID
            && self
                .device
                .find_interface_alt(state, index_high, index_low)
                .is_some_and(|i| u32::from(i.interface_class) == USB_CLASS_PRINTER)
        {
            return Ok(());
        }
        let index = u32::from(index_low);
        match request_type & USB_RECIP_MASK {
            USB_RECIP_ENDPOINT => {
                if (index & !USB_DIR_IN) == 0 {
                    return Ok(());
                }
                // Some applications pass the endpoint number without the direction bit.
                let ifno = self.device.find_interface_for_endpoint(state, index).or_else(|_| {
                    self.device.find_interface_for_endpoint(state, index ^ USB_DIR_IN)
                })?;
                self.check_interface(state, ifno.into())
            }
            USB_RECIP_INTERFACE => self.check_interface(state, index),
            _ => Ok(()),
        }
    }

    fn control(&self, current_task: &CurrentTask, arg: SyscallArg) -> Result<SyscallResult, Errno> {
        let ctrl: usbdevfs_ctrltransfer = current_task.read_multi_arch_object(
            UsbdevfsCtrltransferPtr::new(current_task, UserAddress::from(arg)),
        )?;
        self.check_control_recipient(
            &mut self.device.state.lock(),
            ctrl.bRequestType,
            ctrl.bRequest,
            ctrl.wIndex,
        )?;

        let length = usize::from(ctrl.wLength);
        if length > MAX_USBDEVFS_CONTROL_SIZE {
            return error!(EINVAL);
        }
        let data_addr = UserAddress::from(ctrl.data);
        if length > 0 && data_addr.is_null() {
            return error!(EFAULT);
        }
        let is_in = (u32::from(ctrl.bRequestType) & USB_DIR_IN) != 0;
        let setup = fusb_descriptor::UsbSetup {
            bm_request_type: ctrl.bRequestType,
            b_request: ctrl.bRequest,
            w_value: ctrl.wValue,
            w_index: ctrl.wIndex,
            w_length: ctrl.wLength,
        };
        let timeout = usbfs_timeout(ctrl.timeout);

        if is_in {
            let read_data =
                self.device.control_transfer(current_task, setup, timeout, Vec::new())?;
            let len = min(read_data.len(), length);
            if len > 0 {
                current_task.write_memory(data_addr, &read_data[..len])?;
            }
            Ok(len.into())
        } else {
            let out_data = if length > 0 {
                current_task.read_memory_to_vec(data_addr, length)?
            } else {
                Vec::new()
            };
            self.device.control_transfer(current_task, setup, timeout, out_data)?;
            // The Usb protocol does not report the number of bytes sent by an OUT control
            // transfer: on success, the device accepted all of them.
            Ok(length.into())
        }
    }

    fn bulk(&self, current_task: &CurrentTask, arg: SyscallArg) -> Result<SyscallResult, Errno> {
        let bulk: usbdevfs_bulktransfer = current_task.read_multi_arch_object(
            UsbdevfsBulktransferPtr::new(current_task, UserAddress::from(arg)),
        )?;
        let len = bulk.len as usize;
        let data_addr = UserAddress::from(bulk.data);

        let (client, params, expected_reset) = {
            let mut state = self.device.state.lock();
            let ifno = self.device.find_interface_for_endpoint(&state, bulk.ep)?;
            self.check_interface(&mut state, ifno.into())?;
            if len >= INVALID_USBDEVFS_BULK_SIZE {
                return error!(EINVAL);
            }
            let endpoint = self
                .device
                .active_endpoint(&state, bulk.ep as u8)
                .cloned()
                .ok_or_else(|| errno!(EINVAL))?;
            let params = EndpointTransferParams::for_endpoint(&endpoint)?;
            // usbfs limits the memory used by a transfer.
            if len > MAX_USBDEVFS_BULK_SIZE {
                return error!(ENOMEM);
            }
            if len > 0 && data_addr.is_null() {
                return error!(EFAULT);
            }
            let client = self.device.get_or_connect_endpoint(&mut state, &endpoint)?;
            let expected_reset = client.reset_count();
            (client, params, expected_reset)
        };
        let deadline = deadline_after(usbfs_timeout(bulk.timeout));

        if (bulk.ep & USB_DIR_IN) != 0 {
            let data = self.device.endpoint_bulk_in(
                current_task,
                &client,
                params,
                len,
                deadline,
                expected_reset,
            )?;
            if !data.is_empty() {
                current_task.write_memory(data_addr, &data)?;
            }
            Ok(data.len().into())
        } else {
            let out_data =
                if len > 0 { current_task.read_memory_to_vec(data_addr, len)? } else { Vec::new() };
            let written = self.device.endpoint_bulk_out(
                current_task,
                &client,
                params,
                &out_data,
                deadline,
                expected_reset,
            )?;
            Ok(written.into())
        }
    }

    fn set_interface(
        &self,
        current_task: &CurrentTask,
        arg: SyscallArg,
    ) -> Result<SyscallResult, Errno> {
        let set_iface: usbdevfs_setinterface =
            current_task.read_object(UserRef::new(UserAddress::from(arg)))?;

        let mut state = self.device.state.lock();
        if self.device.active_config(&state).is_none() {
            return error!(EHOSTUNREACH);
        }
        let ifno = Ifno::try_from(set_iface.interface)?;
        if !self.device.has_interface(&state, ifno) {
            return error!(ENOENT);
        }
        // Validate the alternate setting before auto-claiming the interface.
        let alt = u8::try_from(set_iface.altsetting)
            .ok()
            .filter(|&alt| self.device.find_interface_alt(&state, ifno.as_u8(), alt).is_some())
            .ok_or_else(|| errno!(EINVAL))?;
        self.claim_interface(&mut state, ifno)?;

        // Only endpoints of the interface being changed are affected. All of them are disabled,
        // even those also present in the new alternate setting, because the device resets their
        // data toggles. They are enabled again with the new descriptors on next use.
        for ep_addr in self.device.interface_endpoint_addresses(&state, ifno) {
            self.device.disable_endpoint(&mut state, ep_addr)?;
        }
        let proxy = self.device.proxy()?;
        match proxy.set_interface(ifno.as_u8(), alt, zx::MonotonicInstant::INFINITE) {
            // Devices may STALL SET_INTERFACE on an interface with a single alternate setting
            // (USB 2.0 section 9.4.10). Like Linux, treat it as a success and clear the halt of
            // each endpoint of the interface instead, which resets their data toggles.
            Ok(Err(zx::sys::ZX_ERR_IO_REFUSED))
                if self.device.interface_alts(&state, ifno).count() == 1 =>
            {
                for ep_addr in self.device.interface_endpoint_addresses(&state, ifno) {
                    let _ = proxy.control_transfer(
                        &clear_halt_setup(ep_addr),
                        CLEAR_HALT_TIMEOUT.into_nanos(),
                        &[],
                        zx::MonotonicInstant::INFINITE,
                    );
                }
            }
            result => fidl_result_to_errno(result)?,
        }
        state.alt_settings.insert(ifno, alt);
        Ok(SUCCESS)
    }

    fn set_configuration(
        &self,
        current_task: &CurrentTask,
        arg: SyscallArg,
    ) -> Result<SyscallResult, Errno> {
        let raw_config: i32 = current_task.read_object(UserRef::new(UserAddress::from(arg)))?;

        let mut state = self.device.state.lock();
        // Claims are taken under the same lock, so no interface can be claimed concurrently.
        if !state.claims.is_empty() {
            return error!(EBUSY);
        }
        // -1 unconfigures the device, which USB represents as configuration value 0.
        let config = match raw_config {
            -1 => 0,
            value => u8::try_from(value).map_err(|_| errno!(EINVAL))?,
        };
        // Unknown configurations are rejected before touching the device. Whether the device
        // can be unconfigured is up to the USB driver.
        if config != 0 && !self.device.parsed_configs.iter().any(|c| c.config_value == config) {
            return error!(EINVAL);
        }
        self.device.disable_all_endpoints(&mut state)?;
        fidl_result_to_errno(
            self.device.proxy()?.set_configuration(config, zx::MonotonicInstant::INFINITE),
        )?;
        state.active_config = config;
        state.alt_settings.clear();
        Ok(SUCCESS)
    }

    fn clear_halt(
        &self,
        current_task: &CurrentTask,
        arg: SyscallArg,
    ) -> Result<SyscallResult, Errno> {
        let ep: u32 = current_task.read_object(UserRef::new(UserAddress::from(arg)))?;
        let mut state = self.device.state.lock();
        let ifno = self.device.find_interface_for_endpoint(&state, ep)?;
        self.check_interface(&mut state, ifno.into())?;
        let ep_addr = ep as u8;
        // Like Linux usb_clear_halt, the request is not killable: once the device reset its data
        // toggle, the host must reset its own too.
        let proxy = self.device.proxy()?;
        fidl_result_to_errno(proxy.control_transfer(
            &clear_halt_setup(ep_addr),
            CLEAR_HALT_TIMEOUT.into_nanos(),
            &[],
            zx::MonotonicInstant::INFINITE,
        ))?;
        match proxy.reset_endpoint(ep_addr, zx::MonotonicInstant::INFINITE) {
            Ok(Ok(())) => self.device.reset_endpoint_state(ep_addr),
            // The host controller only resets halted endpoints, so it kept the data toggle of
            // this endpoint while the device reset its own. Disable the endpoint so that enabling
            // it again on next use resets the host data toggle too.
            Ok(Err(zx::sys::ZX_ERR_NOT_SUPPORTED)) | Ok(Err(zx::sys::ZX_ERR_INVALID_ARGS)) => {
                self.device.disable_endpoint(&mut state, ep_addr)?
            }
            result => fidl_result_to_errno(result)?,
        }
        Ok(SUCCESS)
    }

    fn get_driver(
        &self,
        current_task: &CurrentTask,
        arg: SyscallArg,
    ) -> Result<SyscallResult, Errno> {
        let user_ref = UserRef::<usbdevfs_getdriver>::new(UserAddress::from(arg));
        let mut getdriver = current_task.read_object(user_ref)?;
        let claimed = {
            let state = self.device.state.lock();
            Ifno::try_from(getdriver.interface).is_ok_and(|ifno| {
                self.device.has_interface(&state, ifno) && state.claims.contains_key(&ifno)
            })
        };
        if !claimed {
            return error!(ENODATA);
        }
        getdriver.driver.fill(0);
        for (dst, src) in getdriver.driver.iter_mut().zip(USBFS_DRIVER_NAME) {
            *dst = *src as _;
        }
        current_task.write_object(user_ref, &getdriver)?;
        Ok(SUCCESS)
    }

    fn disconnect_claim(
        &self,
        current_task: &CurrentTask,
        arg: SyscallArg,
    ) -> Result<SyscallResult, Errno> {
        let disconnect: usbdevfs_disconnect_claim =
            current_task.read_object(UserRef::new(UserAddress::from(arg)))?;
        let mut state = self.device.state.lock();
        let ifno = Ifno::try_from(disconnect.interface)?;
        if !self.device.has_interface(&state, ifno) {
            return error!(EINVAL);
        }
        // Interfaces claimed through usbfs are bound to the "usbfs" driver.
        if state.claims.contains_key(&ifno) {
            let driver: Vec<u8> =
                disconnect.driver.iter().map(|&c| c as u8).take_while(|&c| c != 0).collect();
            let is_usbfs = driver == USBFS_DRIVER_NAME;
            if (disconnect.flags & USBDEVFS_DISCONNECT_CLAIM_IF_DRIVER) != 0 && !is_usbfs {
                return error!(EBUSY);
            }
            if (disconnect.flags & USBDEVFS_DISCONNECT_CLAIM_EXCEPT_DRIVER) != 0 && is_usbfs {
                return error!(EBUSY);
            }
            // Disconnect the current claimant.
            self.device.drop_claim(&mut state, ifno);
        }
        self.claim_interface(&mut state, ifno)?;
        Ok(SUCCESS)
    }

    fn driver_ioctl(
        &self,
        current_task: &CurrentTask,
        arg: SyscallArg,
    ) -> Result<SyscallResult, Errno> {
        let ctl: usbdevfs_ioctl = current_task
            .read_multi_arch_object(UsbdevfsIoctlPtr::new(current_task, UserAddress::from(arg)))?;
        let code = ctl.ioctl_code as u32;
        let size = ((code >> _IOC_SIZESHIFT) & _IOC_SIZEMASK) as usize;
        if size > 0 && ((code >> _IOC_DIRSHIFT) & _IOC_WRITE) != 0 {
            current_task.read_memory_to_vec(UserAddress::from(ctl.data), size)?;
        }
        let mut state = self.device.state.lock();
        if self.device.active_config(&state).is_none() {
            return error!(EHOSTUNREACH);
        }
        let ifno = u32::try_from(ctl.ifno).map_err(|_| errno!(EINVAL)).and_then(Ifno::try_from)?;
        if !self.device.has_interface(&state, ifno) {
            return error!(EINVAL);
        }
        match code {
            USBDEVFS_DISCONNECT => {
                if state.claims.contains_key(&ifno) {
                    self.device.drop_claim(&mut state, ifno);
                    Ok(SUCCESS)
                } else {
                    error!(ENODATA)
                }
            }
            USBDEVFS_CONNECT => {
                if state.claims.contains_key(&ifno) {
                    error!(EBUSY)
                } else {
                    // Starnix has no in-kernel USB interface drivers to bind, so like Linux
                    // `device_attach()` when no driver matches, return 0.
                    Ok(SUCCESS)
                }
            }
            _ => error!(ENOTTY),
        }
    }
}

impl Drop for UsbDeviceFile {
    fn drop(&mut self) {
        let mut state = self.device.state.lock();
        let released: Vec<Ifno> = state
            .claims
            .iter()
            .filter(|&(_, &owner)| owner == self.id)
            .map(|(&ifno, _)| ifno)
            .collect();
        for ifno in released {
            self.device.drop_claim(&mut state, ifno);
        }
    }
}

impl FileOps for UsbDeviceFile {
    fileops_impl_seekable!();
    fileops_impl_noop_sync!();

    fn read(
        &self,
        _file: &FileObject,
        _current_task: &CurrentTask,
        offset: usize,
        data: &mut dyn OutputBuffer,
    ) -> Result<usize, Errno> {
        if self.device.disconnected.load(Ordering::Acquire) {
            return error!(ENODEV);
        }
        let descriptors = &self.device.raw_descriptors;
        if offset >= descriptors.len() {
            return Ok(0);
        }
        data.write(&descriptors[offset..])
    }

    fn write(
        &self,
        _file: &FileObject,
        _current_task: &CurrentTask,
        _offset: usize,
        _data: &mut dyn InputBuffer,
    ) -> Result<usize, Errno> {
        error!(EINVAL)
    }

    fn ioctl(
        &self,
        file: &FileObject,
        current_task: &CurrentTask,
        request: u32,
        arg: SyscallArg,
    ) -> Result<SyscallResult, Errno> {
        // All USBDEVFS_* ioctls require write permission on the open file description.
        if !file.can_write() {
            return error!(EPERM);
        }

        if self.device.disconnected.load(Ordering::Acquire) {
            return error!(ENODEV);
        }

        let request = canonicalize_ioctl_request(current_task.is_arch32(), request);

        match request {
            USBDEVFS_CONTROL => self.control(current_task, arg),
            USBDEVFS_BULK => self.bulk(current_task, arg),
            USBDEVFS_CLAIMINTERFACE => {
                let ifno = read_ifno(current_task, arg)?;
                self.claim_interface(&mut self.device.state.lock(), ifno)?;
                Ok(SUCCESS)
            }
            USBDEVFS_RELEASEINTERFACE => {
                let ifno = read_ifno(current_task, arg)?;
                self.release_interface(&mut self.device.state.lock(), ifno)?;
                Ok(SUCCESS)
            }
            USBDEVFS_SETINTERFACE => self.set_interface(current_task, arg),
            USBDEVFS_SETCONFIGURATION => self.set_configuration(current_task, arg),
            USBDEVFS_CLEAR_HALT => self.clear_halt(current_task, arg),
            USBDEVFS_RESET => {
                // The Usb protocol has no method to reset the physical device. On real Linux,
                // usb_reset_device force-unbinds usbfs interfaces and drops claims because usbfs
                // lacks pre_reset/post_reset callbacks. Since Fuchsia does not perform a hardware
                // reset here, Starnix preserves configuration and claims as a software stub,
                // cancelling in-flight requests.
                track_stub!(
                    TODO("https://fxbug.dev/571954283"),
                    "usbfs USBDEVFS_RESET device reset"
                );
                let _state = self.device.state.lock();
                let addresses: Vec<u8> = self.device.endpoints.lock().keys().copied().collect();
                for ep_addr in addresses {
                    self.device.reset_endpoint_state(ep_addr);
                }
                Ok(SUCCESS)
            }
            USBDEVFS_GETDRIVER => self.get_driver(current_task, arg),
            USBDEVFS_GET_SPEED => Ok((self.device.speed as usize).into()),
            USBDEVFS_GET_CAPABILITIES => {
                // None of the optional capabilities is implemented by the synchronous ioctls.
                let caps: u32 = 0;
                current_task.write_object(UserRef::new(UserAddress::from(arg)), &caps)?;
                Ok(SUCCESS)
            }
            USBDEVFS_CONNECTINFO => {
                let info = usbdevfs_connectinfo {
                    devnum: u32::from(self.device.dev_num),
                    slow: u8::from(self.device.speed == USB_SPEED_LOW),
                    ..Default::default()
                };
                current_task.write_object(UserRef::new(UserAddress::from(arg)), &info)?;
                Ok(SUCCESS)
            }
            USBDEVFS_DISCONNECT_CLAIM => self.disconnect_claim(current_task, arg),
            USBDEVFS_IOCTL => self.driver_ioctl(current_task, arg),
            _ => {
                track_stub!(TODO("https://fxbug.dev/571954283"), "usbfs ioctl", request);
                error!(ENOTTY)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use starnix_uapi::{
        USB_DT_CONFIG_SIZE, USB_DT_INTERFACE_ASSOCIATION_SIZE, USB_DT_INTERFACE_SIZE,
        USB_DT_SS_EP_COMP_SIZE,
    };

    const CONFIG: [u8; USB_DT_CONFIG_SIZE as usize] =
        [USB_DT_CONFIG_SIZE as u8, USB_DT_CONFIG as u8, 0, 0, 1, 7, 0, 0x80, 50];

    fn interface(number: u8, alt: u8) -> [u8; USB_DT_INTERFACE_SIZE as usize] {
        [USB_DT_INTERFACE_SIZE as u8, USB_DT_INTERFACE as u8, number, alt, 1, 0xff, 0, 0, 0]
    }

    fn endpoint(address: u8) -> [u8; USB_DT_ENDPOINT_SIZE as usize] {
        [
            USB_DT_ENDPOINT_SIZE as u8,
            USB_DT_ENDPOINT as u8,
            address,
            USB_ENDPOINT_XFER_BULK as u8,
            0x00,
            0x04,
            0,
        ]
    }

    fn ss_companion(max_burst: u8) -> [u8; USB_DT_SS_EP_COMP_SIZE as usize] {
        [USB_DT_SS_EP_COMP_SIZE as u8, USB_DT_SS_ENDPOINT_COMP as u8, max_burst, 0, 0, 0]
    }

    fn concat(parts: &[&[u8]]) -> Vec<u8> {
        parts.concat()
    }

    #[::fuchsia::test]
    fn test_parse_configuration_interfaces_and_endpoints() {
        let data = concat(&[&CONFIG, &interface(2, 1), &endpoint(0x81)]);
        let configs = parse_config_descriptors(&data);
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].config_value, 7);
        let interface = &configs[0].interfaces[0];
        assert_eq!((interface.interface_number, interface.alt_setting), (2, 1));
        assert_eq!(interface.interface_class, 0xff);
        let endpoint = &interface.endpoints[0];
        assert_eq!(endpoint.address(), 0x81);
        assert_eq!(endpoint.max_packet_size(), 1024);
        assert_eq!(endpoint.ss_companion, NO_SS_COMPANION);
    }

    #[::fuchsia::test]
    fn test_parse_ss_companion_attaches_to_preceding_endpoint() {
        // A class-specific descriptor between an endpoint and its companion is skipped.
        let class_specific = [3, 0x25, 0];
        let data = concat(&[
            &CONFIG,
            &interface(0, 0),
            &endpoint(0x81),
            &class_specific,
            &ss_companion(3),
            &endpoint(0x02),
            &ss_companion(5),
        ]);
        let configs = parse_config_descriptors(&data);
        let endpoints = &configs[0].interfaces[0].endpoints;
        assert_eq!(endpoints[0].ss_companion.b_max_burst, 3);
        assert_eq!(endpoints[0].ss_companion.b_length, USB_DT_SS_EP_COMP_SIZE as u8);
        assert_eq!(endpoints[1].ss_companion.b_max_burst, 5);
    }

    #[::fuchsia::test]
    fn test_parse_ss_companion_ignored_outside_endpoint_scope() {
        let iad = [USB_DT_INTERFACE_ASSOCIATION_SIZE as u8, USB_DT_INTERFACE_ASSOCIATION as u8];
        let iad = concat(&[&iad, &[0; USB_DT_INTERFACE_ASSOCIATION_SIZE as usize - 2]]);
        // A companion after an interface association descriptor, or after an interface
        // descriptor, does not belong to the previous endpoint.
        let data = concat(&[
            &CONFIG,
            &interface(0, 0),
            &endpoint(0x81),
            &iad,
            &ss_companion(3),
            &interface(1, 0),
            &ss_companion(4),
        ]);
        let configs = parse_config_descriptors(&data);
        assert_eq!(configs[0].interfaces.len(), 2);
        let endpoint = &configs[0].interfaces[0].endpoints[0];
        assert_eq!(endpoint.ss_companion, NO_SS_COMPANION);
    }

    #[::fuchsia::test]
    fn test_parse_ignores_descriptors_without_parent() {
        // An interface before any configuration and an endpoint before any interface.
        let data = concat(&[&interface(0, 0), &endpoint(0x81), &CONFIG, &endpoint(0x82)]);
        let configs = parse_config_descriptors(&data);
        assert_eq!(configs.len(), 1);
        assert!(configs[0].interfaces.is_empty());
    }

    #[::fuchsia::test]
    fn test_parse_stops_at_malformed_descriptors() {
        // A descriptor with a bLength smaller than its header ends the parsing.
        let data = concat(&[&CONFIG, &[1, USB_DT_INTERFACE as u8], &interface(0, 0)]);
        assert!(parse_config_descriptors(&data)[0].interfaces.is_empty());

        // So does a truncated descriptor.
        let data = concat(&[&CONFIG, &interface(0, 0)[..5]]);
        assert!(parse_config_descriptors(&data)[0].interfaces.is_empty());
        assert!(parse_config_descriptors(&CONFIG[..1]).is_empty());

        // Descriptors shorter than their standard size are skipped.
        let short_interface = [5, USB_DT_INTERFACE as u8, 0, 0, 0];
        let short_endpoint = [5, USB_DT_ENDPOINT as u8, 0x81, 2, 0];
        let data = concat(&[&CONFIG, &short_interface, &interface(1, 0), &short_endpoint]);
        let configs = parse_config_descriptors(&data);
        assert_eq!(configs[0].interfaces.len(), 1);
        assert_eq!(configs[0].interfaces[0].interface_number, 1);
        assert!(configs[0].interfaces[0].endpoints.is_empty());
    }
}
