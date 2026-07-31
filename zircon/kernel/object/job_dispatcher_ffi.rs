// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use super::job_dispatcher::JobDispatcher;

unsafe extern "C" {
    /// Checks if the given JobDispatcher is the root job dispatcher.
    ///
    /// # Safety
    ///
    /// `job` must point to a valid `JobDispatcher`.
    pub fn cpp_job_dispatcher_is_root(job: *const JobDispatcher) -> bool;
}
