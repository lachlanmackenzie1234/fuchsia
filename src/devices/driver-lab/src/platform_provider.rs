// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! Platform-device resource provider.
//!
//! Maps every MMIO region the bound node offers through
//! `fuchsia.hardware.platform.device` and packages them as a
//! [`ProvidedResources`] bundle for the core. A node that offers no
//! platform device yields an empty bundle: the control plane still
//! serves, but nothing is reachable. A node that offers MMIOs the
//! driver cannot map fails acquisition -- a partially acquired device
//! would misrepresent the resource identity the digests promise.

use fidl_next_fuchsia_hardware_platform_device as fpdev;
use lab_proxy_core::access_policy::{MmioResource, ResourceCeiling};
use lab_proxy_core::hardware_backend::{BackendError, MmioBackend};
use lab_proxy_core::provider::ProvidedResources;
use log::info;
use mmio::Mmio as _;
use mmio::region::MmioRegion;
use mmio::vmo::VmoMemory;
use pdev::PlatformDevice;

/// Provider kind recorded in resource digests.
pub const PROVIDER: &str = "platform";

/// Engineering default ceiling until generated target policy lands:
/// operator-consented unknown reads are permitted and nothing is hard
/// denied. This is the ceiling *content*; the layered enforcement
/// (ceiling, then exact session allowlist, then bounds) is unchanged.
/// TODO: replace with the reviewed policy manifest (spec section 9.6).
fn engineering_ceiling() -> ResourceCeiling {
    ResourceCeiling {
        hard_denied: vec![],
        allow_unknown_reads: true,
        allow_poll: false,
        writable_registers: vec![],
        protocol: None,
        allow_interrupt: false,
    }
}

/// Volatile access to one locally mapped MMIO region.
pub struct MappedMmio {
    region: MmioRegion<VmoMemory>,
}

impl std::fmt::Debug for MappedMmio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MappedMmio").field("len", &self.region.len()).finish()
    }
}

impl MmioBackend for MappedMmio {
    fn read32(&mut self, offset: u64) -> Result<u32, BackendError> {
        let offset = usize::try_from(offset).map_err(|_| BackendError::Fault)?;
        self.region.try_load32(offset).map_err(|_| BackendError::Fault)
    }

    fn write32(&mut self, offset: u64, value: u32) -> Result<(), BackendError> {
        let offset = usize::try_from(offset).map_err(|_| BackendError::Fault)?;
        self.region.try_store32(offset, value).map_err(|_| BackendError::Fault)
    }

    fn barrier(&mut self) {
        std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
    }
}

impl lab_proxy_core::protocol_resource_adapter::ResourceBackend for MappedMmio {
    fn read32(&mut self, offset: u64) -> Result<u32, BackendError> {
        MmioBackend::read32(self, offset)
    }

    fn write32(&mut self, offset: u64, value: u32) -> Result<(), BackendError> {
        MmioBackend::write32(self, offset, value)
    }

    fn barrier(&mut self) {
        MmioBackend::barrier(self);
    }
}

/// Why acquisition failed. Driver start fails on any of these: serving
/// a partially acquired device would break resource-identity promises.
#[derive(Debug)]
pub enum AcquireError {
    /// The device reported an MMIO the driver could not fetch or map.
    Mmio {
        /// The MMIO index that failed.
        id: u32,
    },
}

impl std::fmt::Display for AcquireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AcquireError::Mmio { id } => write!(f, "MMIO {id} could not be fetched or mapped"),
        }
    }
}

fn connect_instance(
    context: &fdf_component::DriverContext,
    instance: &str,
) -> Option<fidl_next::Client<fpdev::Device>> {
    let service = context
        .incoming
        .service::<fdf_component::ServiceInstance<fpdev::Service>>()
        .instance(instance)
        .connect_next()
        .ok()?;
    let (client_end, server_end) = fidl_next::fuchsia::create_channel();
    service.device(server_end).ok()?;
    Some(client_end.spawn())
}

/// Acquires the bound node's platform-device MMIOs.
///
/// Returns an empty bundle when the node offers no platform device (the
/// service is absent or unresponsive) -- by construction no hardware is
/// reachable then. Both common service instance names are tried:
/// `pdev` (the platform bus convention) and `default` (what the SDK
/// offer builders produce).
pub async fn acquire(
    context: &fdf_component::DriverContext,
    node_identity: &str,
) -> Result<ProvidedResources<MappedMmio>, AcquireError> {
    let mut connected = None;
    for instance in ["pdev", "default"] {
        let Some(pdev) = connect_instance(context, instance) else { continue };
        match pdev.get_node_device_info().await {
            Ok(Ok(device_info)) => {
                connected = Some((pdev, device_info.mmio_count.unwrap_or(0)));
                break;
            }
            Ok(Err(status)) => {
                info!("get_node_device_info on instance {instance} returned {status:?}");
            }
            Err(error) => {
                info!("platform device instance {instance} not served ({error:?})");
            }
        }
    }
    let Some((pdev, mmio_count)) = connected else {
        info!("no platform device offered; serving zero resources");
        return Ok(ProvidedResources::empty("none", node_identity));
    };

    let mut bundle = ProvidedResources::empty(PROVIDER, node_identity);
    for id in 0..mmio_count {
        let region = pdev.map_mmio_by_id(id).await.map_err(|error| {
            info!("mapping MMIO {id} failed: {error:?}");
            AcquireError::Mmio { id }
        })?;
        let size = region.len() as u64;
        bundle.resources.insert(id, MmioResource::mmio(format!("mmio{id}"), size, size));
        bundle.ceiling.insert(id, engineering_ceiling());
        bundle.backends.insert(id, MappedMmio { region });
    }
    info!("acquired {mmio_count} platform MMIO resource(s)");
    Ok(bundle)
}
