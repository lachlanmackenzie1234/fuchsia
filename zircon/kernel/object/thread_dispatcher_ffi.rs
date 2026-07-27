// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use super::thread_dispatcher::ThreadDispatcher;
use zx_types::zx_status_t;

unsafe extern "C" {
    /// Checks if the given ThreadDispatcher is the current thread.
    ///
    /// # Safety
    ///
    /// `thread` must point to a valid `ThreadDispatcher`.
    pub fn cpp_thread_dispatcher_is_current(thread: *const ThreadDispatcher) -> bool;

    /// Calls into C++ implementation to suspend a thread.
    ///
    /// # Safety
    ///
    /// `thread` must point to a valid `ThreadDispatcher`.
    pub fn cpp_thread_dispatcher_suspend(thread: *mut ThreadDispatcher) -> zx_status_t;

    /// Calls into C++ implementation to resume a thread.
    ///
    /// # Safety
    ///
    /// `thread` must point to a valid `ThreadDispatcher`.
    pub fn cpp_thread_dispatcher_resume(thread: *mut ThreadDispatcher);

    /// Calls into C++ implementation to set the base profile of a thread.
    ///
    /// # Safety
    ///
    /// `thread` must point to a valid `ThreadDispatcher`.
    /// `profile` must point to a valid `SchedulerStateBaseProfile`.
    pub fn cpp_thread_dispatcher_set_base_profile(
        thread: *mut ThreadDispatcher,
        profile: *const super::thread_dispatcher::SchedulerStateBaseProfile,
    ) -> zx_status_t;

    /// Calls into C++ implementation to set the soft affinity of a thread.
    ///
    /// # Safety
    ///
    /// `thread` must point to a valid `ThreadDispatcher`.
    /// `mask` is the bitmask of CPUs to which the thread can be scheduled.
    pub fn cpp_thread_dispatcher_set_soft_affinity(
        thread: *mut ThreadDispatcher,
        mask: kernel::types::cpu_mask_t,
    ) -> zx_status_t;
}
