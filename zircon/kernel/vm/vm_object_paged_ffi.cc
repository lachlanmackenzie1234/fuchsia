// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include "vm/vm_object_paged_ffi.h"

#include <zircon/types.h>

#include <kernel/ffi.h>

#include "vm/vm_object_paged.h"

// TODO(https://fxbug.dev/537458631): Remove the annotations once cross-language inlining works.
extern "C" {

FFI_ALWAYS_INLINE VmObjectPaged* cpp_vm_object_paged_create(uint32_t pmm_alloc_flags,
                                                            uint32_t options, uint64_t size,
                                                            zx_status_t* out_status) {
  fbl::RefPtr<VmObjectPaged> vmo;
  *out_status = VmObjectPaged::Create(pmm_alloc_flags, options, size, &vmo);
  return fbl::ExportToRawPtr(&vmo);
}

FFI_ALWAYS_INLINE VmObjectPaged* cpp_vm_object_paged_create_contiguous(uint32_t pmm_alloc_flags,
                                                                       uint64_t size,
                                                                       uint8_t alignment_log2,
                                                                       zx_status_t* out_status) {
  fbl::RefPtr<VmObjectPaged> vmo;
  *out_status = VmObjectPaged::CreateContiguous(pmm_alloc_flags, size, alignment_log2, &vmo);
  return fbl::ExportToRawPtr(&vmo);
}

FFI_ALWAYS_INLINE VmObject* cpp_vm_object_paged_as_vm_object(VmObjectPaged* vmo) {
  return static_cast<VmObject*>(vmo);
}

FFI_ALWAYS_INLINE VmCowPages* cpp_vm_object_paged_debug_get_cow_pages(VmObjectPaged* vmo) {
  fbl::RefPtr<VmCowPages> cow = vmo->DebugGetCowPages();
  return fbl::ExportToRawPtr(&cow);
}

FFI_ALWAYS_INLINE VmObjectPaged* cpp_vm_object_as_vm_object_paged(VmObject* vmo) {
  return DownCastVmObject<VmObjectPaged>(vmo);
}

FFI_ALWAYS_INLINE vm_page_t* cpp_vm_object_paged_debug_get_page(VmObjectPaged* vmo,
                                                                uint64_t offset) {
  return vmo->DebugGetPage(offset);
}

}  // extern "C"
