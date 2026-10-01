// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use crate::counters::define_kcounter;
use crate::kernel::deadline::{Deadline, InstantUnknown};
use crate::kernel::thread::AutoExpiringPreemptDisabler;
use crate::object::{
    ChannelDispatcher, HandleOwner, HandleRef, HandleTableWriteGuard, HandleValue,
    MAX_MESSAGE_HANDLES, MessagePacket, MessagePacketPtr, ProcessDispatcher, ThreadDispatcher,
    UserHandles, remove_user_handles,
};
use crate::user_copy::{UserInOutPtr, UserInPtr, UserOutPtr};
use core::mem::{MaybeUninit, align_of, size_of};
use core::ptr::{self, NonNull};
use debug::ltracef;
use syscalls_macro::syscall;
use zerocopy::{FromBytes, Immutable, IntoBytes};
use zr::static_assert_size_and_align;
use zx_status::Status;
use zx_types::{
    ZX_CHANNEL_READ_MAY_DISCARD, ZX_CHANNEL_WRITE_USE_IOVEC, ZX_POL_NEW_CHANNEL,
    zx_channel_call_args_t, zx_channel_call_etc_args_t, zx_channel_iovec_t,
    zx_handle_disposition_t, zx_handle_info_t, zx_handle_t, zx_instant_mono_t, zx_obj_type_t,
    zx_rights_t, zx_txid_t,
};

const LOCAL_TRACE: u32 = 0;

define_kcounter!(CHANNEL_MSG_0_BYTES, "channel.bytes.0", Sum);
define_kcounter!(CHANNEL_MSG_64_BYTES, "channel.bytes.64", Sum);
define_kcounter!(CHANNEL_MSG_256_BYTES, "channel.bytes.256", Sum);
define_kcounter!(CHANNEL_MSG_1K_BYTES, "channel.bytes.1k", Sum);
define_kcounter!(CHANNEL_MSG_4K_BYTES, "channel.bytes.4k", Sum);
define_kcounter!(CHANNEL_MSG_16K_BYTES, "channel.bytes.16k", Sum);
define_kcounter!(CHANNEL_MSG_64K_BYTES, "channel.bytes.64k", Sum);
define_kcounter!(CHANNEL_MSG_0_HANDLES, "channel.handles.00", Sum);
define_kcounter!(CHANNEL_MSG_1_HANDLES, "channel.handles.01", Sum);
define_kcounter!(CHANNEL_MSG_2_HANDLES, "channel.handles.02", Sum);
define_kcounter!(CHANNEL_MSG_4_HANDLES, "channel.handles.04", Sum);
define_kcounter!(CHANNEL_MSG_8_HANDLES, "channel.handles.08", Sum);
define_kcounter!(CHANNEL_MSG_16_HANDLES, "channel.handles.16", Sum);
define_kcounter!(CHANNEL_MSG_32_HANDLES, "channel.handles.32", Sum);
define_kcounter!(CHANNEL_MSG_64_HANDLES, "channel.handles.64", Sum);
define_kcounter!(CHANNEL_MSG_RECEIVED, "channel.messages", Sum);

fn record_recv_msg_sz(num_bytes: u32, num_handles: u32) {
    CHANNEL_MSG_RECEIVED.add(1);

    match num_bytes {
        0 => CHANNEL_MSG_0_BYTES.add(1),
        1..=64 => CHANNEL_MSG_64_BYTES.add(1),
        65..=256 => CHANNEL_MSG_256_BYTES.add(1),
        257..=1024 => CHANNEL_MSG_1K_BYTES.add(1),
        1025..=4096 => CHANNEL_MSG_4K_BYTES.add(1),
        4097..=16384 => CHANNEL_MSG_16K_BYTES.add(1),
        16385..=65536 => CHANNEL_MSG_64K_BYTES.add(1),
        _ => {}
    }

    match num_handles {
        0 => CHANNEL_MSG_0_HANDLES.add(1),
        1 => CHANNEL_MSG_1_HANDLES.add(1),
        2 => CHANNEL_MSG_2_HANDLES.add(1),
        3..=4 => CHANNEL_MSG_4_HANDLES.add(1),
        5..=8 => CHANNEL_MSG_8_HANDLES.add(1),
        9..=16 => CHANNEL_MSG_16_HANDLES.add(1),
        17..=32 => CHANNEL_MSG_32_HANDLES.add(1),
        33..=64 => CHANNEL_MSG_64_HANDLES.add(1),
        _ => {}
    }
}

#[repr(C)]
#[derive(Copy, Clone, Default, FromBytes, IntoBytes, Immutable)]
struct RawHandleInfo {
    handle: zx_handle_t,
    type_: zx_obj_type_t,
    rights: zx_rights_t,
    unused: u32,
}

static_assert_size_and_align!(
    RawHandleInfo,
    size_of::<zx_handle_info_t>(),
    align_of::<zx_handle_info_t>(),
);

#[repr(C)]
#[derive(Copy, Clone, Default, FromBytes, IntoBytes, Immutable)]
struct RawChannelCallArgs {
    wr_bytes: usize,
    wr_handles: usize,
    rd_bytes: usize,
    rd_handles: usize,
    wr_num_bytes: u32,
    wr_num_handles: u32,
    rd_num_bytes: u32,
    rd_num_handles: u32,
}

static_assert_size_and_align!(
    RawChannelCallArgs,
    size_of::<zx_channel_call_args_t>(),
    align_of::<zx_channel_call_args_t>()
);

static_assert_size_and_align!(
    RawChannelCallArgs,
    size_of::<zx_channel_call_etc_args_t>(),
    align_of::<zx_channel_call_etc_args_t>()
);

#[syscall]
pub fn sys_channel_create(
    options: u32,
    out0: &mut HandleValue,
    out1: &mut HandleValue,
) -> Result<(), Status> {
    if options != 0 {
        return Err(Status::INVALID_ARGS);
    }

    let up = ProcessDispatcher::get_current();
    up.enforce_basic_policy(ZX_POL_NEW_CHANNEL)?;

    let (handle0, handle1, rights) = ChannelDispatcher::create()?;

    *out0 = up.make_and_add_handle(handle0, rights)?;
    *out1 = up.make_and_add_handle(handle1, rights)?;
    Ok(())
}

fn map_handle_to_value(up: &ProcessDispatcher, handle: HandleRef<'_>, out: &mut u32) {
    *out = up.map_handle_to_value(handle).raw_value();
}

fn map_handle_to_value_info(
    up: &ProcessDispatcher,
    handle: HandleRef<'_>,
    out: &mut RawHandleInfo,
) {
    let disp = handle.dispatcher_ref();
    out.handle = up.map_handle_to_value(handle).raw_value();
    out.type_ = disp.get_type();
    out.rights = handle.rights();
    out.unused = 0;
}

trait ReadHandle: IntoBytes + Immutable + Copy + Default {
    fn map_handle_to_value(up: &ProcessDispatcher, handle: HandleRef<'_>, out: &mut Self);
}

impl ReadHandle for u32 {
    #[inline]
    fn map_handle_to_value(up: &ProcessDispatcher, handle: HandleRef<'_>, out: &mut Self) {
        map_handle_to_value(up, handle, out);
    }
}

impl ReadHandle for RawHandleInfo {
    #[inline]
    fn map_handle_to_value(up: &ProcessDispatcher, handle: HandleRef<'_>, out: &mut Self) {
        map_handle_to_value_info(up, handle, out);
    }
}

// Removes the handles from `msg`, install them in `up`'s handle table, and copies them out to the
// user array `handles`.
//
// Upon completion, the Handle object will either be owned by the process (success) or closed
// (error).
fn msg_get_handles<HandleT: ReadHandle>(
    up: &ProcessDispatcher,
    msg: &mut MessagePacket,
    handles: UserOutPtr<HandleT>,
    num_handles: u32,
) -> Result<(), Status> {
    let handle_list = msg.handles();
    let num_handles = num_handles as usize;

    let mut hvs = [HandleT::default(); MAX_MESSAGE_HANDLES];
    for (i, hv) in hvs[..num_handles].iter_mut().enumerate() {
        // SAFETY: `i < num_handles` and `msg` owns `num_handles` valid non-null `Handle*` pointers.
        let handle_ref =
            unsafe { HandleRef::from_raw(NonNull::new_unchecked(*handle_list.add(i))) };
        HandleT::map_handle_to_value(up, handle_ref, hv);
    }

    handles.copy_slice_to_user(&hvs[..num_handles])?;

    // The MessagePacket currently owns the handle. Only after transferring the handles into this
    // process's handle table can we relieve MessagePacket of its handle ownership responsibility.
    for i in 0..num_handles {
        // SAFETY: `i < num_handles` and `msg` owns `num_handles` valid non-null `Handle*` pointers.
        let handle_ref =
            unsafe { HandleRef::from_raw(NonNull::new_unchecked(*handle_list.add(i))) };
        let disp = handle_ref.dispatcher_ref();
        if disp.is_waitable() {
            // Cancel any waiters on this handle prior to adding it to the process's handle table.
            disp.cancel(handle_ref);
            // If this handle refers to a channel, cancel any channel_call waits.
            if let Some(channel) = disp.downcast::<ChannelDispatcher>() {
                channel.cancel_message_waiters();
            }
        }
    }

    {
        // TODO(https://fxbug.dev/42105832): This takes a lock per call. Consider doing these in a
        // batch.
        let _preempt_disable = AutoExpiringPreemptDisabler::with_default_timeslice_extension();
        ksync::lock!(let guard = HandleTableWriteGuard::new(up));
        for i in 0..num_handles {
            // SAFETY: `i < num_handles`, and ownership of each `Handle*` is transferred from `msg`
            // to `up`'s handle table (`msg.set_owns_handles(false)` is called below).
            let handle = unsafe { HandleOwner::from_raw(*handle_list.add(i)).unwrap() };
            guard.add_handle(handle);
        }
    }
    msg.set_owns_handles(false);

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn channel_read<HandleInfoT: ReadHandle>(
    handle_value: HandleValue,
    options: u32,
    bytes: UserOutPtr<u8>,
    handles: UserOutPtr<HandleInfoT>,
    mut num_bytes: u32,
    mut num_handles: u32,
    actual_bytes: UserOutPtr<u32>,
    actual_handles: UserOutPtr<u32>,
) -> Result<(), Status> {
    ltracef!(
        "handle {:#x} bytes {:?} num_bytes {:?} handles {:?} num_handles {:?}\n",
        handle_value.raw_value(),
        bytes.as_ptr(),
        actual_bytes.as_ptr(),
        handles.as_ptr(),
        actual_handles.as_ptr()
    );

    let up = ProcessDispatcher::get_current();

    let channel =
        up.get_dispatcher_with_rights::<ChannelDispatcher>(handle_value, zx_types::ZX_RIGHT_READ)?;

    // Currently MAY_DISCARD is the only allowable option.
    if (options & !ZX_CHANNEL_READ_MAY_DISCARD) != 0 {
        return Err(Status::NOT_SUPPORTED);
    }

    let result = channel.read(
        up.handle_table_koid(),
        &mut num_bytes,
        &mut num_handles,
        (options & ZX_CHANNEL_READ_MAY_DISCARD) != 0,
    );
    if let Err(err) = result
        && err != Status::BUFFER_TOO_SMALL
    {
        return Err(err);
    }

    // On ZX_ERR_BUFFER_TOO_SMALL, Read() gives us the size of the next message (which remains
    // unconsumed, unless `options` has ZX_CHANNEL_READ_MAY_DISCARD set).
    if !actual_bytes.is_null() {
        actual_bytes.write(num_bytes)?;
    }

    if !actual_handles.is_null() {
        actual_handles.write(num_handles)?;
    }
    let mut msg = result?;

    if num_bytes > 0 && msg.copy_data_to(bytes).is_err() {
        return Err(Status::INVALID_ARGS);
    }

    // The documented public API states that that writing to the handles buffer
    // must happen after writing to the data buffer.
    if num_handles > 0 {
        msg_get_handles(&up, &mut msg, handles, num_handles)?;
    }

    record_recv_msg_sz(num_bytes, num_handles);
    Ok(())
}

#[syscall]
pub fn sys_channel_read(
    handle_value: HandleValue,
    options: u32,
    bytes: UserOutPtr<u8>,
    handle_info: UserOutPtr<zx_handle_t>,
    num_bytes: u32,
    num_handles: u32,
    actual_bytes: UserOutPtr<u32>,
    actual_handles: UserOutPtr<u32>,
) -> Result<(), Status> {
    channel_read(
        handle_value,
        options,
        bytes,
        handle_info,
        num_bytes,
        num_handles,
        actual_bytes,
        actual_handles,
    )
}

#[syscall]
pub fn sys_channel_read_etc(
    handle_value: HandleValue,
    options: u32,
    bytes: UserOutPtr<u8>,
    handle_info: UserOutPtr<zx_handle_info_t>,
    num_bytes: u32,
    num_handles: u32,
    actual_bytes: UserOutPtr<u32>,
    actual_handles: UserOutPtr<u32>,
) -> Result<(), Status> {
    channel_read(
        handle_value,
        options,
        bytes,
        handle_info.reinterpret::<RawHandleInfo>(),
        num_bytes,
        num_handles,
        actual_bytes,
        actual_handles,
    )
}

fn channel_read_out<HandleInfoT: ReadHandle>(
    up: &ProcessDispatcher,
    mut reply: MessagePacketPtr,
    args: &RawChannelCallArgs,
    actual_bytes: UserOutPtr<u32>,
    actual_handles: UserOutPtr<u32>,
) -> Result<(), Status> {
    let num_bytes = reply.data_size() as u32;
    let num_handles = reply.num_handles() as u32;

    if (args.rd_num_bytes < num_bytes) || (args.rd_num_handles < num_handles) {
        return Err(Status::BUFFER_TOO_SMALL);
    }

    actual_bytes.write(num_bytes)?;
    actual_handles.write(num_handles)?;

    if num_bytes > 0 {
        let rd_bytes = UserOutPtr::<u8>::new(ptr::with_exposed_provenance_mut(args.rd_bytes));
        if reply.copy_data_to(rd_bytes).is_err() {
            return Err(Status::INVALID_ARGS);
        }
    }

    if num_handles > 0 {
        let rd_handles =
            UserOutPtr::<HandleInfoT>::new(ptr::with_exposed_provenance_mut(args.rd_handles));
        msg_get_handles(up, &mut reply, rd_handles, num_handles)?;
    }
    Ok(())
}

fn channel_call_epilogue<HandleInfoT: ReadHandle>(
    up: &ProcessDispatcher,
    reply: MessagePacketPtr,
    args: &RawChannelCallArgs,
    actual_bytes: UserOutPtr<u32>,
    actual_handles: UserOutPtr<u32>,
) -> Result<(), Status> {
    let bytes = reply.data_size() as u32;
    let handles = reply.num_handles() as u32;
    channel_read_out::<HandleInfoT>(up, reply, args, actual_bytes, actual_handles)?;
    record_recv_msg_sz(bytes, handles);
    Ok(())
}

// For zx_handle_write or zx_handle_write_etc with the ZX_HANDLE_OP_MOVE flag,
// handles are closed whether success or failure. For zx_handle_write_etc
// with the ZX_HANDLE_OP_DUPLICATE flag, handles always remain open.
pub(crate) fn msg_put_handles<T: UserHandles>(
    up: &ProcessDispatcher,
    msg: &mut MessagePacket,
    user_handles: T,
    num_handles: u32,
    channel: &ChannelDispatcher,
) -> Result<(), Status> {
    let num_handles = num_handles as usize;
    debug_assert!(num_handles <= MAX_MESSAGE_HANDLES); // This must be checked before calling.

    let mut handles_buf = [MaybeUninit::<T::ValueType>::uninit(); MAX_MESSAGE_HANDLES];
    let handles = user_handles.copy_array_from_user(&mut handles_buf[..num_handles])?;

    let mut status = Ok(());
    {
        let _preempt_disable = AutoExpiringPreemptDisabler::with_default_timeslice_extension();
        ksync::lock!(let guard = HandleTableWriteGuard::new(up));

        let mutable_handles = msg.handles_mut();
        for (ix, handle_entry) in handles.iter_mut().enumerate() {
            let handle = T::get_handle_for_message_locked(&guard, channel, handle_entry);
            if let Err(err) = handle
                && status.is_ok()
            {
                // Latch the first error encountered. It will be what the function returns.
                status = Err(err);
            }

            // SAFETY: `ix < num_handles` and `mutable_handles` has space for `num_handles` pointers.
            unsafe {
                *mutable_handles.add(ix) = handle.map_or(ptr::null_mut(), HandleOwner::release);
            }
        }
    }

    // For zx_handle_write_etc, copy out to convey zx_status_t result on failure. The caller
    // is expected to have initialized the result to ZX_OK (mentioned in the user docs)
    // to save cycles for the success case.
    if T::IS_OUT
        && status.is_err()
        && let Err(copy_status) = user_handles.copy_array_to_user(handles)
    {
        status = Err(copy_status);
    }

    msg.set_owns_handles(true);
    status
}

fn channel_write<T: UserHandles>(
    handle_value: HandleValue,
    options: u32,
    user_bytes: UserInPtr<u8>,
    num_bytes: u32,
    user_handles: T,
    num_handles: u32,
) -> Result<(), Status> {
    ltracef!(
        "handle {:#x} bytes {:?} num_bytes {} handles {:?} num_handles {} options {:#x}\n",
        handle_value.raw_value(),
        user_bytes.as_ptr(),
        num_bytes,
        ptr::from_ref(&user_handles),
        num_handles,
        options
    );

    let up = ProcessDispatcher::get_current();

    let mut cleanup = zr::defer(|| {
        let _ = remove_user_handles(user_handles, num_handles as usize, &up);
    });

    if (options & !ZX_CHANNEL_WRITE_USE_IOVEC) != 0 {
        return Err(Status::INVALID_ARGS);
    }

    let channel =
        up.get_dispatcher_with_rights::<ChannelDispatcher>(handle_value, zx_types::ZX_RIGHT_WRITE)?;

    let mut msg = if (options & ZX_CHANNEL_WRITE_USE_IOVEC) != 0 {
        MessagePacket::create_from_iovecs(
            user_bytes.reinterpret::<zx_channel_iovec_t>(),
            num_bytes as usize,
            num_handles as usize,
        )?
    } else {
        MessagePacket::create_from_user(user_bytes, num_bytes as usize, num_handles as usize)?
    };

    // msg_put_handles() always consumes all handles that should be consumed (or
    // there are zero handles, and so there's nothing to be done).
    cleanup.cancel();

    if num_handles > 0 {
        msg_put_handles(&up, &mut msg, user_handles, num_handles, &channel)?;
    }

    channel.write(up.handle_table_koid(), msg)
}

fn channel_call_noretry<WriteHandlesT: UserHandles, ReadHandleT: ReadHandle>(
    handle_value: HandleValue,
    options: u32,
    deadline: zx_instant_mono_t,
    user_args: UserInPtr<RawChannelCallArgs>,
    actual_bytes: UserOutPtr<u32>,
    actual_handles: UserOutPtr<u32>,
    make_wr_handles: fn(usize) -> WriteHandlesT,
) -> Result<(), Status> {
    let args = user_args.read()?;

    let user_bytes = UserInPtr::<u8>::new(ptr::with_exposed_provenance(args.wr_bytes));
    let user_handles = make_wr_handles(args.wr_handles);

    let num_bytes = args.wr_num_bytes;
    let num_handles = args.wr_num_handles;

    let up = ProcessDispatcher::get_current();

    let mut cleanup = zr::defer(|| {
        let _ = remove_user_handles(user_handles, num_handles as usize, &up);
    });

    if (options & !ZX_CHANNEL_WRITE_USE_IOVEC) != 0 {
        return Err(Status::INVALID_ARGS);
    }

    let channel = up.get_dispatcher_with_rights::<ChannelDispatcher>(
        handle_value,
        zx_types::ZX_RIGHT_WRITE | zx_types::ZX_RIGHT_READ,
    )?;

    // Prepare a MessagePacket for writing
    let mut msg = if (options & ZX_CHANNEL_WRITE_USE_IOVEC) != 0 {
        MessagePacket::create_from_iovecs(
            user_bytes.reinterpret::<zx_channel_iovec_t>(),
            num_bytes as usize,
            num_handles as usize,
        )?
    } else {
        MessagePacket::create_from_user(user_bytes, num_bytes as usize, num_handles as usize)?
    };

    if msg.data_size() < size_of::<zx_txid_t>() {
        return Err(Status::INVALID_ARGS);
    }

    // msg_put_handles() always consumes all handles (or there are zero handles,
    // and so there's nothing to be done).
    cleanup.cancel();

    if num_handles > 0 {
        msg_put_handles(&up, &mut msg, user_handles, num_handles, &channel)?;
    }

    // Write message and wait for reply, deadline, or cancellation
    let reply = channel.call(up.handle_table_koid(), msg, deadline)?;
    channel_call_epilogue::<ReadHandleT>(&up, reply, &args, actual_bytes, actual_handles)
}

fn channel_call_finish<ReadHandleT: ReadHandle>(
    deadline: zx_instant_mono_t,
    user_args: UserInPtr<RawChannelCallArgs>,
    actual_bytes: UserOutPtr<u32>,
    actual_handles: UserOutPtr<u32>,
) -> Result<(), Status> {
    let args = user_args.read()?;

    let up = ProcessDispatcher::get_current();

    let waiter = ThreadDispatcher::get_current_message_waiter();
    let channel = waiter.get_channel().ok_or(Status::BAD_STATE)?;

    let slack = up.get_timer_slack_policy();
    let slack_deadline = Deadline::new(InstantUnknown(deadline), slack);
    let reply = channel.resume_interrupted_call(&waiter, slack_deadline)?;
    channel_call_epilogue::<ReadHandleT>(&up, reply, &args, actual_bytes, actual_handles)
}

#[syscall]
pub fn sys_channel_write(
    handle_value: HandleValue,
    options: u32,
    user_bytes: UserInPtr<u8>,
    num_bytes: u32,
    user_handles: UserInPtr<zx_handle_t>,
    num_handles: u32,
) -> Result<(), Status> {
    ltracef!(
        "handle {:#x} bytes {:?} num_bytes {} handles {:?} num_handles {} options {:#x}\n",
        handle_value.raw_value(),
        user_bytes.as_ptr(),
        num_bytes,
        user_handles.as_ptr(),
        num_handles,
        options
    );

    channel_write(handle_value, options, user_bytes, num_bytes, user_handles, num_handles)
}

#[syscall]
pub fn sys_channel_write_etc(
    handle_value: HandleValue,
    options: u32,
    user_bytes: UserInPtr<u8>,
    num_bytes: u32,
    user_handles: UserInOutPtr<zx_handle_disposition_t>,
    num_handles: u32,
) -> Result<(), Status> {
    ltracef!(
        "handle {:#x} bytes {:?} num_bytes {} handles {:?} num_handles {} options {:#x}\n",
        handle_value.raw_value(),
        user_bytes.as_ptr(),
        num_bytes,
        user_handles.as_in_ptr().as_ptr(),
        num_handles,
        options
    );

    channel_write(handle_value, options, user_bytes, num_bytes, user_handles, num_handles)
}

#[syscall]
pub fn sys_channel_call_noretry(
    handle_value: HandleValue,
    options: u32,
    deadline: zx_instant_mono_t,
    user_args: UserInPtr<zx_channel_call_args_t>,
    actual_bytes: UserOutPtr<u32>,
    actual_handles: UserOutPtr<u32>,
) -> Result<(), Status> {
    channel_call_noretry::<UserInPtr<zx_handle_t>, u32>(
        handle_value,
        options,
        deadline,
        user_args.reinterpret::<RawChannelCallArgs>(),
        actual_bytes,
        actual_handles,
        |addr| UserInPtr::new(ptr::with_exposed_provenance(addr)),
    )
}

#[syscall]
pub fn sys_channel_call_finish(
    deadline: zx_instant_mono_t,
    user_args: UserInPtr<zx_channel_call_args_t>,
    actual_bytes: UserOutPtr<u32>,
    actual_handles: UserOutPtr<u32>,
) -> Result<(), Status> {
    channel_call_finish::<u32>(
        deadline,
        user_args.reinterpret::<RawChannelCallArgs>(),
        actual_bytes,
        actual_handles,
    )
}

#[syscall]
pub fn sys_channel_call_etc_noretry(
    handle_value: HandleValue,
    options: u32,
    deadline: zx_instant_mono_t,
    user_args: UserInOutPtr<zx_channel_call_etc_args_t>,
    actual_bytes: UserOutPtr<u32>,
    actual_handles: UserOutPtr<u32>,
) -> Result<(), Status> {
    channel_call_noretry::<UserInOutPtr<zx_handle_disposition_t>, RawHandleInfo>(
        handle_value,
        options,
        deadline,
        user_args.reinterpret::<RawChannelCallArgs>().as_in_ptr(),
        actual_bytes,
        actual_handles,
        |addr| UserInOutPtr::new(ptr::with_exposed_provenance_mut(addr)),
    )
}

#[syscall]
pub fn sys_channel_call_etc_finish(
    deadline: zx_instant_mono_t,
    user_args: UserInOutPtr<zx_channel_call_etc_args_t>,
    actual_bytes: UserOutPtr<u32>,
    actual_handles: UserOutPtr<u32>,
) -> Result<(), Status> {
    channel_call_finish::<RawHandleInfo>(
        deadline,
        user_args.reinterpret::<RawChannelCallArgs>().as_in_ptr(),
        actual_bytes,
        actual_handles,
    )
}
