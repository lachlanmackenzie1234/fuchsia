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

// TODO(https://fxbug.dev/537458631): Remove the annotations once cross-language inlining works.
FFI_ALWAYS_INLINE uint64_t cpp_vm_object_size(const VmObject* vmo) { return vmo->size(); }

// TODO(https://fxbug.dev/537458631): Remove the annotations once cross-language inlining works.
FFI_ALWAYS_INLINE bool cpp_vm_object_is_resizable(const VmObject* vmo) {
  return vmo->is_resizable();
}

// TODO(https://fxbug.dev/537458631): Remove the annotations once cross-language inlining works.
FFI_ALWAYS_INLINE zx_status_t cpp_vm_object_resize(VmObject* vmo, uint64_t size) {
  return vmo->Resize(size);
}

// TODO(https://fxbug.dev/537458631): Remove the annotations once cross-language inlining works.
FFI_ALWAYS_INLINE zx_status_t cpp_vm_object_write(VmObject* vmo, const void* ptr, uint64_t offset,
                                                  size_t len) {
  return vmo->Write(ptr, offset, len);
}

// TODO(https://fxbug.dev/537458631): Remove the annotations once cross-language inlining works.
FFI_ALWAYS_INLINE zx_status_t cpp_vm_object_set_name(VmObject* vmo, const char* name, size_t len) {
  return vmo->set_name(name, len);
}

// TODO(https://fxbug.dev/537458631): Remove the annotations once cross-language inlining works.
FFI_ALWAYS_INLINE zx_status_t cpp_vm_object_decommit_range(VmObject* vmo, uint64_t offset,
                                                           uint64_t len) {
  if (!vmo) {
    return ZX_ERR_INVALID_ARGS;
  }
  return vmo->DecommitRange(offset, len);
}

}  // extern "C"
