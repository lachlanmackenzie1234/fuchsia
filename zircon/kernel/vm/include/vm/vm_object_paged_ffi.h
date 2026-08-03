// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#ifndef ZIRCON_KERNEL_VM_INCLUDE_VM_VM_OBJECT_PAGED_FFI_H_
#define ZIRCON_KERNEL_VM_INCLUDE_VM_VM_OBJECT_PAGED_FFI_H_

#include <zircon/compiler.h>
#include <zircon/types.h>

#include "vm/vm_object_paged.h"

__BEGIN_CDECLS

VmObjectPaged* cpp_vm_object_paged_create(uint32_t pmm_alloc_flags, uint32_t options, uint64_t size,
                                          zx_status_t* out_status);
VmObjectPaged* cpp_vm_object_paged_create_contiguous(uint32_t pmm_alloc_flags, uint64_t size,
                                                     uint8_t alignment_log2,
                                                     zx_status_t* out_status);
VmObject* cpp_vm_object_paged_as_vm_object(VmObjectPaged* vmo);
VmCowPages* cpp_vm_object_paged_debug_get_cow_pages(VmObjectPaged* vmo);
void* cpp_vm_object_paged_debug_get_page(VmObjectPaged* vmo, uint64_t offset);

__END_CDECLS

#endif  // ZIRCON_KERNEL_VM_INCLUDE_VM_VM_OBJECT_PAGED_FFI_H_
