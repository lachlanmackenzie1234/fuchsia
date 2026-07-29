// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use super::event_dispatcher::{EventDispatcher, EventDispatcherState};
use super::handle::KernelHandle;
use zx_types::zx_status_t;

// C++ FFI declarations
unsafe extern "C" {
    pub(crate) fn cpp_event_dispatcher_create(
        options: u32,
        handle_out: *mut KernelHandle<EventDispatcher>,
    ) -> zx_status_t;
}

// FFI trampolines for C++ calling into Rust EventDispatcherState

crate::object::dispatcher::impl_dispatcher_state_init!(EventDispatcher, EventDispatcherState);

/// FFI trampoline for creating an EventDispatcher from C++.
///
/// # Safety
///
/// `rights_out` and `handle_out` must point to valid, non-null, writable memory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_event_dispatcher_create(
    options: u32,
    rights_out: *mut zx_types::zx_rights_t,
    handle_out: *mut KernelHandle<EventDispatcher>,
) -> zx_types::zx_status_t {
    // SAFETY: `rights_out` and `handle_out` are valid non-null writable pointers.
    unsafe {
        match EventDispatcher::create(options) {
            Ok((handle, rights)) => {
                rights_out.write(rights);
                handle_out.write(handle);
                zx_types::ZX_OK
            }
            Err(status) => status.into_raw(),
        }
    }
}
