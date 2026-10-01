// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use super::channel_dispatcher::ChannelDispatcher;
use super::handle::{HandleOwner, HandleRef, HandleValue};
use super::process_dispatcher::{HandleTableWriteGuard, ProcessDispatcher};
use crate::user_copy::{UserInOutPtr, UserInPtr};
use core::mem::{MaybeUninit, align_of, size_of};
use core::ops::Deref;
use core::{cmp, ptr};
use zerocopy::{FromBytes, Immutable, IntoBytes};
use zx_status::Status;
use zx_types::{
    ZX_CHANNEL_MAX_MSG_HANDLES, ZX_HANDLE_INVALID, ZX_HANDLE_OP_DUPLICATE, ZX_HANDLE_OP_MOVE,
    ZX_OBJ_TYPE_NONE, ZX_RIGHT_DUPLICATE, ZX_RIGHT_SAME_RIGHTS, ZX_RIGHT_TRANSFER,
    zx_handle_disposition_t, zx_handle_op_t, zx_handle_t, zx_obj_type_t, zx_rights_t, zx_status_t,
};

pub const MAX_MESSAGE_HANDLES: usize = ZX_CHANNEL_MAX_MSG_HANDLES as usize;

#[repr(C)]
#[derive(Copy, Clone, Default, FromBytes, IntoBytes, Immutable)]
pub struct RawHandleDisposition {
    pub operation: zx_handle_op_t,
    pub handle: zx_handle_t,
    pub type_: zx_obj_type_t,
    pub rights: zx_rights_t,
    pub result: zx_status_t,
}

zr::static_assert_size_and_align!(
    RawHandleDisposition,
    size_of::<zx_handle_disposition_t>(),
    align_of::<zx_handle_disposition_t>(),
);

// Basic checks for a `handle` to be able to be sent via `channel`.
fn common_handle_checks_locked(
    handle: HandleRef<'_>,
    channel: &ChannelDispatcher,
    desired_rights: zx_rights_t,
    type_: zx_obj_type_t,
) -> Result<(), Status> {
    if !handle.has_rights(ZX_RIGHT_TRANSFER) {
        return Err(Status::ACCESS_DENIED);
    }
    if ptr::eq(handle.dispatcher_ref(), channel.deref()) {
        return Err(Status::NOT_SUPPORTED);
    }
    if type_ != ZX_OBJ_TYPE_NONE && handle.dispatcher_ref().get_type() != type_ {
        return Err(Status::WRONG_TYPE);
    }
    if desired_rights != ZX_RIGHT_SAME_RIGHTS
        && (handle.rights() & desired_rights) != desired_rights
    {
        return Err(Status::INVALID_ARGS);
    }
    Ok(())
}

fn move_handle_for_transfer(
    mut handle: HandleOwner,
    channel: &ChannelDispatcher,
    type_: zx_obj_type_t,
    desired_rights: zx_rights_t,
) -> Result<HandleOwner, Status> {
    common_handle_checks_locked(handle.as_ref(), channel, desired_rights, type_)?;

    // If the caller has requested a different set of rights, we have to mint a new handle for them.
    if desired_rights != ZX_RIGHT_SAME_RIGHTS && desired_rights != handle.rights() {
        // common_handle_checks_locked verifies that the desired rights are a subset of the handle's
        // current rights.
        let descoped_rights_handle = handle.dup(desired_rights).map_err(|_| Status::NO_MEMORY)?;
        handle = descoped_rights_handle;
    }

    Ok(handle)
}

fn duplicate_handle_for_transfer(
    source: HandleRef<'_>,
    channel: &ChannelDispatcher,
    type_: zx_obj_type_t,
    desired_rights: zx_rights_t,
) -> Result<HandleOwner, Status> {
    common_handle_checks_locked(source, channel, desired_rights, type_)?;

    if !source.has_rights(ZX_RIGHT_DUPLICATE) {
        return Err(Status::ACCESS_DENIED);
    }

    // common_handle_checks_locked verifies that the desired rights are a subset of the handle's
    // current rights.
    let dest_rights =
        if desired_rights == ZX_RIGHT_SAME_RIGHTS { source.rights() } else { desired_rights };

    let duped_handle = source.dup(dest_rights).map_err(|_| Status::NO_MEMORY)?;

    Ok(duped_handle)
}

// Extracts the handles that would be consumed on syscalls with handle_release semantics
// from `offset` to `offset + chunk_size` from `user_handles` and returns them
// in `handles` which must be at of at least size `chunk_size`.
pub fn get_user_handles_to_consume(
    user_handles: UserInPtr<zx_handle_t>,
    offset: usize,
    chunk_size: usize,
    handles: &mut [zx_handle_t],
) -> Result<(), Status> {
    let mut local_handles = [MaybeUninit::<zx_handle_t>::uninit(); MAX_MESSAGE_HANDLES];
    let copied = user_handles
        .element_offset(offset)
        .copy_slice_from_user(&mut local_handles[..chunk_size])?;
    handles[..chunk_size].copy_from_slice(copied);
    Ok(())
}

pub fn get_user_handles_to_consume_disposition(
    user_handles: UserInOutPtr<zx_handle_disposition_t>,
    offset: usize,
    mut chunk_size: usize,
    handles: &mut [zx_handle_t],
) -> Result<(), Status> {
    let mut local_handle_disposition =
        [MaybeUninit::<RawHandleDisposition>::uninit(); MAX_MESSAGE_HANDLES];

    chunk_size = cmp::min(chunk_size, MAX_MESSAGE_HANDLES);

    let local_handle_disposition = user_handles
        .reinterpret::<RawHandleDisposition>()
        .as_in_ptr()
        .element_offset(offset)
        .copy_slice_from_user(&mut local_handle_disposition[..chunk_size])?;

    for i in 0..chunk_size {
        // !ZX_HANDLE_OP_DUPLICATE is used to capture the case where we failed
        // due to a bad operational arg.
        if local_handle_disposition[i].operation != ZX_HANDLE_OP_DUPLICATE {
            handles[i] = local_handle_disposition[i].handle;
        }
    }
    Ok(())
}

// Removes the handles pointed by `user_handles` from `process`. Returns Ok(()) if all handles
// have been removed, error otherwise. It only stops early if get_user_handles() fails.
pub fn remove_user_handles<T: UserHandles>(
    user_handles: T,
    num_handles: usize,
    process: &ProcessDispatcher,
) -> Result<(), Status> {
    let mut handles = [ZX_HANDLE_INVALID; MAX_MESSAGE_HANDLES];
    let mut offset = 0;
    let mut status = Ok(());

    while offset < num_handles {
        // We process `num_handles` in chunks of `MAX_MESSAGE_HANDLES` because we don't have
        // a limit on how large `num_handles` can be.
        let chunk_size = cmp::min(num_handles - offset, MAX_MESSAGE_HANDLES);
        if let Err(consume_status) =
            user_handles.get_user_handles_to_consume(offset, chunk_size, &mut handles)
        {
            status = Err(consume_status);
            break;
        }

        if let Err(remove_status) = process.remove_handles(&handles[..chunk_size]) {
            status = Err(remove_status);
        }
        offset += chunk_size;
    }
    status
}

// Returns a handle that should be sent over `channel`. In case
// of error, the return value should be reflected back to the user.
// This overload is used by zx_channel_write.
pub fn get_handle_for_message_locked(
    guard: &HandleTableWriteGuard<'_>,
    channel: &ChannelDispatcher,
    handle_val: &mut zx_handle_t,
) -> Result<HandleOwner, Status> {
    let source = guard.remove_handle(HandleValue::new(*handle_val)).ok_or(Status::BAD_HANDLE)?;
    move_handle_for_transfer(source, channel, ZX_OBJ_TYPE_NONE, ZX_RIGHT_SAME_RIGHTS)
}

// This overload is used by zx_channel_write_etc.
pub fn get_handle_for_message_locked_disposition(
    guard: &HandleTableWriteGuard<'_>,
    channel: &ChannelDispatcher,
    handle_disposition: &mut RawHandleDisposition,
) -> Result<HandleOwner, Status> {
    let operation = handle_disposition.operation;
    let desired_rights = handle_disposition.rights;
    let type_ = handle_disposition.type_;
    let handle_val = handle_disposition.handle;

    let operation_result = (|| -> Result<HandleOwner, Status> {
        let source = guard.get_handle(HandleValue::new(handle_val)).ok_or(Status::BAD_HANDLE)?;

        // The documentation for zx_channel_write_etc says this about the operation performed on
        // handles:
        // The operation applied to *handle* is one of:
        //
        // *   `ZX_HANDLE_OP_MOVE` This is equivalent to first issuing [`zx_handle_replace()`] then
        //      [`zx_channel_write()`]. The source handle is always closed.
        //
        // *   `ZX_HANDLE_OP_DUPLICATE` This is equivalent to first issuing [`zx_handle_duplicate()`]
        //     then [`zx_channel_write()`]. The source handle always remains open and accessible to
        //     the caller.
        // So when duplicating a handle, we leave the source handle in the handle table. For all other
        // operations (including invalid operations) we immediately remove the source handle from the
        // handle table and then attempt to move it.

        if operation == ZX_HANDLE_OP_DUPLICATE {
            return duplicate_handle_for_transfer(source, channel, type_, desired_rights);
        }

        let source_owner = guard.remove_handle_ref(source);
        if operation == ZX_HANDLE_OP_MOVE {
            return move_handle_for_transfer(source_owner, channel, type_, desired_rights);
        }
        Err(Status::INVALID_ARGS)
    })();

    if let Err(err) = operation_result {
        handle_disposition.result = err.into_raw();
    }
    operation_result
}

pub trait UserHandles: Copy {
    type ValueType: FromBytes + IntoBytes + Immutable + Copy + Default;
    const IS_OUT: bool;

    fn get_user_handles_to_consume(
        self,
        offset: usize,
        chunk_size: usize,
        handles: &mut [zx_handle_t],
    ) -> Result<(), Status>;

    fn get_handle_for_message_locked(
        guard: &HandleTableWriteGuard<'_>,
        channel: &ChannelDispatcher,
        handle_entry: &mut Self::ValueType,
    ) -> Result<HandleOwner, Status>;

    fn copy_array_from_user(
        self,
        dst: &mut [MaybeUninit<Self::ValueType>],
    ) -> Result<&mut [Self::ValueType], Status>;

    fn copy_array_to_user(self, src: &[Self::ValueType]) -> Result<(), Status>;
}

impl UserHandles for UserInPtr<zx_handle_t> {
    type ValueType = zx_handle_t;
    const IS_OUT: bool = false;

    #[inline]
    fn get_user_handles_to_consume(
        self,
        offset: usize,
        chunk_size: usize,
        handles: &mut [zx_handle_t],
    ) -> Result<(), Status> {
        get_user_handles_to_consume(self, offset, chunk_size, handles)
    }

    #[inline]
    fn get_handle_for_message_locked(
        guard: &HandleTableWriteGuard<'_>,
        channel: &ChannelDispatcher,
        handle_entry: &mut zx_handle_t,
    ) -> Result<HandleOwner, Status> {
        get_handle_for_message_locked(guard, channel, handle_entry)
    }

    #[inline]
    fn copy_array_from_user(
        self,
        dst: &mut [MaybeUninit<zx_handle_t>],
    ) -> Result<&mut [zx_handle_t], Status> {
        self.copy_slice_from_user(dst)
    }

    #[inline]
    fn copy_array_to_user(self, _src: &[zx_handle_t]) -> Result<(), Status> {
        Ok(())
    }
}

impl UserHandles for UserInOutPtr<zx_handle_disposition_t> {
    type ValueType = RawHandleDisposition;
    const IS_OUT: bool = true;

    #[inline]
    fn get_user_handles_to_consume(
        self,
        offset: usize,
        chunk_size: usize,
        handles: &mut [zx_handle_t],
    ) -> Result<(), Status> {
        get_user_handles_to_consume_disposition(self, offset, chunk_size, handles)
    }

    #[inline]
    fn get_handle_for_message_locked(
        guard: &HandleTableWriteGuard<'_>,
        channel: &ChannelDispatcher,
        handle_entry: &mut RawHandleDisposition,
    ) -> Result<HandleOwner, Status> {
        get_handle_for_message_locked_disposition(guard, channel, handle_entry)
    }

    #[inline]
    fn copy_array_from_user(
        self,
        dst: &mut [MaybeUninit<RawHandleDisposition>],
    ) -> Result<&mut [RawHandleDisposition], Status> {
        self.reinterpret::<RawHandleDisposition>().as_in_ptr().copy_slice_from_user(dst)
    }

    #[inline]
    fn copy_array_to_user(self, src: &[RawHandleDisposition]) -> Result<(), Status> {
        self.reinterpret::<RawHandleDisposition>().copy_slice_to_user(src)
    }
}
