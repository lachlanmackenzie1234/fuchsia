// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use super::vm_address_region_dispatcher::VmAddressRegionDispatcher;
use zx_types::zx_status_t;

unsafe extern "C" {
    /// Calls into C++ implementation to set the memory priority of a VMAR.
    ///
    /// # Safety
    ///
    /// `vmar` must point to a valid `VmAddressRegionDispatcher`.
    /// `priority` is the memory priority to set.
    pub fn cpp_vmar_dispatcher_set_memory_priority(
        vmar: *mut VmAddressRegionDispatcher,
        priority: u32,
    ) -> zx_status_t;
}
