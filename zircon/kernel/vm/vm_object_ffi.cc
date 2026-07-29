// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include "vm/vm_object_ffi.h"

#include <zircon/types.h>

#include "vm/vm_object.h"

extern "C" {

void* cpp_vm_object_get_ref_counted(const VmObject* vmo) {
  return const_cast<fbl::RefCountedUpgradeable<VmObject>*>(
      static_cast<const fbl::RefCountedUpgradeable<VmObject>*>(vmo));
}

void cpp_vm_object_free(VmObject* vmo) { delete vmo; }

}  // extern "C"
