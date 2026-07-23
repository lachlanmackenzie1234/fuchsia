// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use crate::thread_dispatcher::ThreadDispatcher;
use zx_types::zx_status_t;

// C++ FFI declarations
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
}
