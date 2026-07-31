// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use super::job_dispatcher_ffi::cpp_job_dispatcher_is_root;

crate::object::dispatcher::impl_dispatcher_facade!(
    pub struct JobDispatcher,
    zx_types::ZX_OBJ_TYPE_JOB
);

impl JobDispatcher {
    /// Returns whether this `JobDispatcher` is the root job.
    pub fn is_root(&self) -> bool {
        // SAFETY: `self` is a valid `JobDispatcher` reference.
        unsafe { cpp_job_dispatcher_is_root(self as *const _) }
    }
}
