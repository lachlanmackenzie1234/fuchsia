// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#ifndef ZIRCON_KERNEL_VM_INCLUDE_VM_VM_OBJECT_FFI_H_
#define ZIRCON_KERNEL_VM_INCLUDE_VM_VM_OBJECT_FFI_H_

#include <zircon/compiler.h>
#include <zircon/types.h>

#include "vm/vm_object.h"

__BEGIN_CDECLS

void* cpp_vm_object_get_ref_counted(const VmObject* vmo);
void cpp_vm_object_free(VmObject* vmo);

__END_CDECLS

#endif  // ZIRCON_KERNEL_VM_INCLUDE_VM_VM_OBJECT_FFI_H_
