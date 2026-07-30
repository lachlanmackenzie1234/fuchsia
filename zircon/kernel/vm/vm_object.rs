// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

unsafe extern "C" {
    fn cpp_vm_object_get_ref_counted(vmo: *const VmObject) -> *mut fbl::RefCounted;
    fn cpp_vm_object_free(vmo: *mut VmObject);
}

fbl::impl_opaque_ref_counted_facade!(
    /// The base vm object that holds a range of bytes of data
    ///
    /// Can be created without mapping and used as a container of data, or mappable
    /// into an address space via VmAddressRegion::CreateVmMapping
    pub struct VmObject,
    cpp_vm_object_free,
    cpp_vm_object_get_ref_counted,
);
