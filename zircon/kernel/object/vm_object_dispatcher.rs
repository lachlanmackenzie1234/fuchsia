// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use super::vm_object_dispatcher_ffi::cpp_vm_object_dispatcher_get_vmo;
use crate::vm::vm_object::VmObject;
use fbl::RefPtr;
use zx_types::ZX_OBJ_TYPE_VMO;

crate::object::dispatcher::impl_dispatcher_facade!(
    pub struct VmObjectDispatcher,
    ZX_OBJ_TYPE_VMO
);

impl VmObjectDispatcher {
    /// Returns a reference to the underlying `VmObject`.
    pub fn vmo(&self) -> &RefPtr<VmObject> {
        // SAFETY: `self` is a valid `VmObjectDispatcher` reference.
        unsafe { &*cpp_vm_object_dispatcher_get_vmo(self as *const _) }
    }
}
