// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use crate::dispatcher::Dispatcher;
use crate::handle::{HandleValue, KernelHandle};
use crate::process_dispatcher::ProcessDispatcher;
use zx_types::{zx_rights_t, zx_status_t};

unsafe extern "C" {
    /// Returns a raw pointer to the current process dispatcher.
    ///
    /// # Safety
    ///
    /// The caller must only call this when executing within a valid thread context.
    pub(crate) fn cpp_process_dispatcher_current() -> *const ProcessDispatcher;

    /// Checks if the given ProcessDispatcher is the current process.
    ///
    /// # Safety
    ///
    /// `process` must point to a valid `ProcessDispatcher`.
    pub(crate) fn cpp_process_dispatcher_is_current(process: *const ProcessDispatcher) -> bool;

    /// Calls into C++ implementation to suspend a process.
    ///
    /// # Safety
    ///
    /// `process` must point to a valid `ProcessDispatcher`.
    pub(crate) fn cpp_process_dispatcher_suspend(process: *mut ProcessDispatcher) -> zx_status_t;

    /// Calls into C++ implementation to resume a process.
    ///
    /// # Safety
    ///
    /// `process` must point to a valid `ProcessDispatcher`.
    pub(crate) fn cpp_process_dispatcher_resume(process: *mut ProcessDispatcher);

    /// Creates and adds a handle to the given process.
    ///
    /// # Safety
    ///
    /// `process` must point to a valid `ProcessDispatcher`.
    /// `handle` must point to a valid `KernelHandle<Dispatcher>`.
    /// `out_handle` must point to writable memory.
    pub(crate) fn cpp_process_dispatcher_make_and_add_handle(
        process: *const ProcessDispatcher,
        handle: *mut KernelHandle<Dispatcher>,
        rights: zx_rights_t,
        out_handle: *mut HandleValue,
    ) -> zx_status_t;

    /// Retrieves a dispatcher and rights from the handle table of the current process.
    ///
    /// Upon success, the `out_dispatcher` argument is initialized by C++ to contain a
    /// `fbl::RefPtr<Dispatcher>` pointing to the dispatcher associated with the given handle.
    /// The caller typically uses `MaybeUninit::zeroed()` to initialize `out_dispatcher` and
    /// checks the return status to determine if C++ initialized the value.
    ///
    /// # Safety
    ///
    /// `out_dispatcher` must point to memory for a `fbl::RefPtr<Dispatcher>` that is initialized
    /// by the caller.  Usually this will be zeroed memory, but pointers to valid
    /// `fbl::RefPtr<Dispatcher>` values are also acceptable.
    /// `out_rights` must point to writable memory.
    pub(crate) fn cpp_handle_table_get_dispatcher(
        handle: HandleValue,
        out_dispatcher: *mut fbl::RefPtr<Dispatcher>,
        out_rights: *mut zx_rights_t,
    ) -> zx_status_t;

    /// Enforces basic policy for the given process.
    ///
    /// # Safety
    ///
    /// `process` must point to a valid `ProcessDispatcher`.
    pub(crate) fn cpp_process_dispatcher_enforce_basic_policy(
        process: *const ProcessDispatcher,
        policy: u32,
    ) -> zx_status_t;
}
