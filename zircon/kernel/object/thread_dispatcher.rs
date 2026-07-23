// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use crate::thread_dispatcher_ffi::{
    cpp_thread_dispatcher_is_current, cpp_thread_dispatcher_resume, cpp_thread_dispatcher_suspend,
};
use zx_status::Status;

crate::impl_dispatcher_facade!(
    pub struct ThreadDispatcher,
    zx_types::ZX_OBJ_TYPE_THREAD
);

impl ThreadDispatcher {
    /// Returns whether this `ThreadDispatcher` is the current thread.
    pub fn is_current(&self) -> bool {
        // SAFETY: `self` is a valid `ThreadDispatcher` reference.
        unsafe { cpp_thread_dispatcher_is_current(self as *const _) }
    }

    /// Suspends execution of this thread.
    ///
    /// # Errors
    ///
    /// - `ZX_ERR_BAD_STATE` if the thread is dying or dead.
    pub fn suspend(&self) -> Result<(), Status> {
        // SAFETY: `self` is a valid `ThreadDispatcher` reference.
        let status = unsafe { cpp_thread_dispatcher_suspend(self as *const _ as *mut _) };
        Status::ok(status)
    }

    /// Resumes execution of this thread.
    pub fn resume(&self) {
        // SAFETY: `self` is a valid `ThreadDispatcher` reference.
        unsafe { cpp_thread_dispatcher_resume(self as *const _ as *mut _) }
    }
}
