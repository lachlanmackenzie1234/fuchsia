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
FFI_ALWAYS_INLINE bool cpp_vm_object_is_contiguous(const VmObject* vmo) {
  return vmo->is_contiguous();
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

// TODO(https://fxbug.dev/537458631): Remove the annotations once cross-language inlining works.
FFI_ALWAYS_INLINE zx_status_t cpp_vm_object_commit_range_pinned(VmObject* vmo, uint64_t offset,
                                                                uint64_t len, bool write) {
  return vmo->CommitRangePinned(offset, len, write);
}

// TODO(https://fxbug.dev/537458631): Remove the annotations once cross-language inlining works.
FFI_ALWAYS_INLINE void cpp_vm_object_unpin(VmObject* vmo, uint64_t offset, uint64_t len) {
  vmo->Unpin(offset, len);
}

zx_status_t cpp_vm_object_hint_range(VmObject* vmo, uint64_t offset, uint64_t len,
                                     VmObject::EvictionHint hint) {
  if (!vmo) {
    return ZX_ERR_INVALID_ARGS;
  }
  return vmo->HintRange(offset, len, hint);
}

uint8_t cpp_vm_object_get_mapping_cache_policy(const VmObject* vmo) {
  return vmo->GetMappingCachePolicy();
}

VmObject* cpp_vm_object_create_clone(VmObject* vmo, Resizability resizable,
                                     SnapshotType snapshot_type, uint64_t offset, uint64_t size,
                                     bool copy_name, zx_status_t* out_status) {
  fbl::RefPtr<VmObject> child;
  zx_status_t status = vmo->CreateClone(resizable, snapshot_type, offset, size, copy_name, &child);
  if (out_status) {
    *out_status = status;
  }
  return fbl::ExportToRawPtr(&child);
}

zx_status_t cpp_vm_object_get_page_blocking(VmObject* vmo, uint64_t offset, uint32_t pf_flags) {
  return vmo->GetPageBlocking(offset, pf_flags, nullptr, nullptr, nullptr);
}

void cpp_vm_object_set_user_id(VmObject* vmo, uint64_t user_id) { vmo->set_user_id(user_id); }

uint64_t cpp_vm_object_user_id(const VmObject* vmo) { return vmo->user_id(); }

uint64_t cpp_vm_object_parent_user_id(const VmObject* vmo) { return vmo->parent_user_id(); }

zx_status_t cpp_vm_object_lookup(VmObject* vmo, uint64_t offset, uint64_t len, void* ctx,
                                 cpp_vm_object_lookup_fn callback) {
  return vmo->Lookup(offset, len, [ctx, callback](uint64_t offset, paddr_t pa) {
    return callback(ctx, offset, pa);
  });
}

}  // extern "C"
