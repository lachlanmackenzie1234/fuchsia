// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use super::vm_object_dispatcher::VmObjectDispatcher;
use crate::vm::vm_object::VmObject;
use fbl::RefPtr;

unsafe extern "C" {
    pub(crate) fn cpp_vm_object_dispatcher_get_vmo(
        disp: *const VmObjectDispatcher,
    ) -> *const RefPtr<VmObject>;
}
