// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use super::log_dispatcher::{LogDispatcher, LogDispatcherState};
use crate::DispatcherOps;
use crate::handle::KernelHandle;

use zx_types::{ZX_LOG_READABLE, zx_rights_t, zx_status_t};

// C++ FFI declarations
unsafe extern "C" {
    pub(crate) fn cpp_log_dispatcher_create(
        flags: u32,
        rights: zx_rights_t,
        handle_out: *mut KernelHandle<LogDispatcher>,
    ) -> zx_status_t;
}

// Trampoline callbacks from C++ into Rust LogDispatcherState

/// # Safety
///
/// `ptr` must point to uninitialized memory of at least `size_of::<LogDispatcherState>()` bytes,
/// and `dispatcher` must point to the enclosing `LogDispatcher`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_log_dispatcher_state_init(
    ptr: *mut LogDispatcherState,
    dispatcher: *const LogDispatcher,
    flags: u32,
) {
    // SAFETY: `ptr` points to uninitialized memory allocated for `LogDispatcherState`.
    unsafe {
        let _ = pin_init::PinInit::__pinned_init(LogDispatcherState::init(dispatcher, flags), ptr);
    }
}

/// # Safety
///
/// The caller must ensure `state` is a valid reference to an initialized `LogDispatcherState`,
/// and must not use the state (or the enclosing dispatcher) after this function returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_log_dispatcher_state_destroy(state: &mut LogDispatcherState) {
    // SAFETY: The caller is destroying the dispatcher and will not use it again.
    unsafe {
        core::ptr::drop_in_place(state);
    }
}

/// Trampoline callback for DlogReader notify.
///
/// # Safety
///
/// `cookie` must point to the `LogDispatcher`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_log_dispatcher_notify(cookie: *mut core::ffi::c_void) {
    // SAFETY: `cookie` was passed during `DlogReader` initialization and
    // points to the `LogDispatcher`.
    unsafe {
        let dispatcher = cookie.cast::<LogDispatcher>();
        (*dispatcher).update_state(0, ZX_LOG_READABLE);
    }
}

/// Trampoline callback for LogDispatcher creation from C++.
///
/// # Safety
///
/// `rights_out` and `handle_out` must be valid writable pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_log_dispatcher_create(
    flags: u32,
    rights_out: *mut zx_rights_t,
    handle_out: *mut KernelHandle<LogDispatcher>,
) -> zx_status_t {
    // SAFETY: `rights_out` and `handle_out` are valid non-null writable pointers.
    unsafe {
        match LogDispatcher::create(flags) {
            Ok((handle, rights)) => {
                rights_out.write(rights);
                handle_out.write(handle);
                zx_types::ZX_OK
            }
            Err(status) => status.into_raw(),
        }
    }
}
