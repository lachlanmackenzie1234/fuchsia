// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use physical_page_borrowing_config_bindings as bindings;

/// RAII guard to enable or disable page loaning for a scope, restoring the previous state on drop.
pub struct ScopedLoaningEnabled {
    prev_state: bool,
}

impl ScopedLoaningEnabled {
    /// Creates a new `ScopedLoaningEnabled` guard, enabling or disabling page loaning for its
    /// lifetime.
    pub fn new(enable: bool) -> Self {
        let prev_state = unsafe { bindings::cpp_set_loaning_enabled(enable) };
        Self { prev_state }
    }
}

impl Drop for ScopedLoaningEnabled {
    fn drop(&mut self) {
        unsafe { bindings::cpp_set_loaning_enabled(self.prev_state) };
    }
}
