// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use super::dispatcher::DispatcherOps;
use super::handle::{HandleValue, KernelHandle};
use super::process_dispatcher_ffi::{
    cpp_process_dispatcher_current, cpp_process_dispatcher_enforce_basic_policy,
    cpp_process_dispatcher_is_current, cpp_process_dispatcher_make_and_add_handle,
    cpp_process_dispatcher_resume, cpp_process_dispatcher_suspend,
};
use zx_status::Status;
use zx_types::zx_rights_t;

crate::object::dispatcher::impl_dispatcher_facade!(
    pub struct ProcessDispatcher,
    zx_types::ZX_OBJ_TYPE_PROCESS
);

impl ProcessDispatcher {
    /// Executes the given function with a reference to the current process.
    pub fn with_current<R>(f: impl FnOnce(&ProcessDispatcher) -> R) -> R {
        // SAFETY: The current process is guaranteed to be valid for the duration of the call.
        let proc = unsafe { &*cpp_process_dispatcher_current() };
        f(proc)
    }

    /// Returns whether this `ProcessDispatcher` is the current process.
    pub fn is_current(&self) -> bool {
        // SAFETY: `self` is a valid `ProcessDispatcher` reference.
        unsafe { cpp_process_dispatcher_is_current(self as *const _) }
    }

    /// Suspends execution of this process.
    ///
    /// # Errors
    ///
    /// - `ZX_ERR_BAD_STATE` if the process is dying or dead.
    pub fn suspend(&self) -> Result<(), Status> {
        // SAFETY: `self` is a valid `ProcessDispatcher` reference.
        let status = unsafe { cpp_process_dispatcher_suspend(self as *const _ as *mut _) };
        Status::ok(status)
    }

    /// Resumes execution of this process.
    pub fn resume(&self) {
        // SAFETY: `self` is a valid `ProcessDispatcher` reference.
        unsafe { cpp_process_dispatcher_resume(self as *const _ as *mut _) }
    }

    /// Creates a handle for the given dispatcher in this process's handle table.
    pub fn make_and_add_handle<T>(
        &self,
        handle: KernelHandle<T>,
        rights: zx_rights_t,
    ) -> Result<HandleValue, Status>
    where
        T: fbl::HasRefCount + fbl::Recyclable + DispatcherOps,
    {
        let mut handle = handle.cast();
        let mut out = HandleValue::default();
        // SAFETY: `self` is a valid `ProcessDispatcher`, `handle` is a valid `KernelHandle`, and
        // `out` points to writable memory.
        let status = unsafe {
            cpp_process_dispatcher_make_and_add_handle(
                self as *const _,
                &mut handle,
                rights,
                &mut out,
            )
        };
        Status::ok(status)?;
        Ok(out)
    }

    /// Creates a handle for the given dispatcher reference in this process's handle table.
    pub fn make_and_add_handle_from_ref<T>(
        &self,
        dispatcher: fbl::RefPtr<T>,
        rights: zx_rights_t,
    ) -> Result<HandleValue, Status>
    where
        T: fbl::HasRefCount + fbl::Recyclable + DispatcherOps,
    {
        // SAFETY: T implements DispatcherOps and is layout-compatible with Dispatcher.
        let raw_dispatcher =
            fbl::RefPtr::into_raw(unsafe { dispatcher.cast::<super::Dispatcher>() });
        let mut out = HandleValue::default();
        // SAFETY: `self` is a valid `ProcessDispatcher`, `raw_dispatcher` carries an acquired reference count
        // transferred to C++, and `out` points to writable memory.
        let status = unsafe {
            super::process_dispatcher_ffi::cpp_process_dispatcher_make_and_add_handle_from_ref(
                self as *const _,
                raw_dispatcher,
                rights,
                &mut out,
            )
        };
        Status::ok(status)?;
        Ok(out)
    }

    /// Enforces basic policy for this process.
    pub fn enforce_basic_policy(&self, policy: u32) -> Result<(), Status> {
        // SAFETY: `self` is a valid `ProcessDispatcher` reference.
        let status =
            unsafe { cpp_process_dispatcher_enforce_basic_policy(self as *const _, policy) };
        Status::ok(status)
    }
}
