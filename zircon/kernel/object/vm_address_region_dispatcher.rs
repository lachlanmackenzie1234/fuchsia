// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use super::vm_address_region_dispatcher_ffi::cpp_vmar_dispatcher_set_memory_priority;
use zx_status::Status;

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryPriority {
    Default = 0,
    High = 1,
}

crate::object::dispatcher::impl_dispatcher_facade!(
    pub struct VmAddressRegionDispatcher,
    zx_types::ZX_OBJ_TYPE_VMAR
);

impl VmAddressRegionDispatcher {
    /// Sets memory priority for this VMAR dispatcher.
    pub fn set_memory_priority(&self, priority: MemoryPriority) -> Result<(), Status> {
        // SAFETY: `self` is a valid `VmAddressRegionDispatcher` reference.
        let status = unsafe {
            cpp_vmar_dispatcher_set_memory_priority(self as *const _ as *mut _, priority as u32)
        };
        Status::ok(status)
    }
}
