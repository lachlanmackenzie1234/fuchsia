// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use core::mem::MaybeUninit;

use crate::object::{HandleValue, validate_system_resource};
use syscalls_macro::syscall;
use user_copy::UserOutPtr;
use zx_status::{ErrorStatus, Status};
use zx_types::*;

// Allocate this many extra bytes at the end of the bootdata for the platform
// to fill in with platform specific boot structures.
pub const BOOTDATA_PLATFORM_EXTRA_BYTES: usize = page::SIZE * 4;

unsafe extern "C" {
    fn cpp_system_mexec_payload_get_helper(
        buffer: *mut MaybeUninit<u8>,
        buffer_size: usize,
        out_zbi_size: *mut usize,
    ) -> zx_status_t;
    fn cpp_system_mexec_core(
        resource: zx_handle_t,
        kernel_vmo: zx_handle_t,
        data_zbi_vmo: zx_handle_t,
    ) -> zx_status_t;
}

#[syscall]
pub fn sys_system_mexec_payload_get(
    resource: HandleValue,
    user_buffer: UserOutPtr<u8>,
    buffer_size: usize,
) -> Result<(), ErrorStatus> {
    if !boot_options_rs::enable_debugging_syscalls() {
        return Err(Status::NOT_SUPPORTED.into());
    }
    // Highly privileged, only mexec resource should have access.
    validate_system_resource(resource, ZX_RSRC_SYSTEM_MEXEC_BASE)?;

    // Limit the size of the result that we can return to userspace.
    if buffer_size > BOOTDATA_PLATFORM_EXTRA_BYTES {
        return Err(Status::INVALID_ARGS.into());
    }

    let mut buffer =
        kalloc::Box::<[u8]>::try_new_uninit_slice(buffer_size).map_err(|_| Status::NO_MEMORY)?;
    let mut zbi_size = 0usize;
    // SAFETY: We pass a valid allocated buffer of size `buffer_size` and a valid `zbi_size` out pointer.
    let status = unsafe {
        cpp_system_mexec_payload_get_helper(buffer.as_mut_ptr(), buffer_size, &mut zbi_size)
    };
    Status::ok(status)?;
    debug_assert!(zbi_size <= buffer_size);
    // SAFETY: cpp_system_mexec_payload_get_helper initializes buffer[..zbi_size] on success.
    let zbi_buffer = unsafe { core::slice::from_raw_parts(buffer.as_ptr() as *const u8, zbi_size) };
    user_buffer.copy_slice_to_user(&zbi_buffer)?;
    Ok(())
}

#[syscall]
pub fn sys_system_mexec(
    resource: HandleValue,
    kernel_vmo: HandleValue,
    data_zbi_vmo: HandleValue,
) -> Result<(), ErrorStatus> {
    if !boot_options_rs::enable_debugging_syscalls() {
        return Err(Status::NOT_SUPPORTED.into());
    }
    validate_system_resource(resource, ZX_RSRC_SYSTEM_MEXEC_BASE)?;

    // SAFETY: Forwarding handles to inner C++ mexec coalescing & execution logic.
    let status = unsafe {
        cpp_system_mexec_core(
            resource.raw_value(),
            kernel_vmo.raw_value(),
            data_zbi_vmo.raw_value(),
        )
    };
    Status::ok(status)?;
    Ok(())
}
