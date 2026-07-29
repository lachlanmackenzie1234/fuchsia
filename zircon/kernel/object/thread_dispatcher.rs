// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use super::thread_dispatcher_ffi::{
    cpp_thread_dispatcher_is_current, cpp_thread_dispatcher_resume, cpp_thread_dispatcher_suspend,
};
use object_constants_rs as object_constants;
use zx_status::Status;

pub use crate::kernel::types::cpu_mask_t;

/// Opaque byte container matching C++ `SchedulerState::BaseProfile`.
#[repr(C, align(8))]
pub struct SchedulerStateBaseProfile(
    pub zr::OpaqueBytes<{ object_constants::kSchedulerStateBaseProfileSize }>,
);

zr::static_assert_size_and_align!(
    SchedulerStateBaseProfile,
    object_constants::kSchedulerStateBaseProfileSize,
    object_constants::kSchedulerStateBaseProfileAlign,
);

impl SchedulerStateBaseProfile {
    /// Returns a raw pointer to the underlying byte storage.
    pub fn get(&self) -> *mut [u8; object_constants::kSchedulerStateBaseProfileSize] {
        self.0.get()
    }
}

crate::object::dispatcher::impl_dispatcher_facade!(
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

    /// Sets the base profile for this thread.
    pub fn set_base_profile(&self, profile: &SchedulerStateBaseProfile) -> Result<(), Status> {
        // SAFETY: `self` is a valid `ThreadDispatcher` reference and `profile` points to
        // an opaque `SchedulerState::BaseProfile`.
        let status = unsafe {
            super::thread_dispatcher_ffi::cpp_thread_dispatcher_set_base_profile(
                self as *const _ as *mut _,
                profile.get() as *const _,
            )
        };
        Status::ok(status)
    }

    /// Sets soft CPU affinity for this thread.
    pub fn set_soft_affinity(&self, mask: cpu_mask_t) -> Result<(), Status> {
        // SAFETY: `self` is a valid `ThreadDispatcher` reference.
        let status = unsafe {
            super::thread_dispatcher_ffi::cpp_thread_dispatcher_set_soft_affinity(
                self as *const _ as *mut _,
                mask,
            )
        };
        Status::ok(status)
    }
}
