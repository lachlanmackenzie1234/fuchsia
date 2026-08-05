// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include "vm/vm_cow_pages_ffi.h"

#include <zircon/types.h>

#include "vm/vm_cow_pages.h"

extern "C" {

zx_status_t cpp_vm_cow_pages_replace_page_with_loaned(VmCowPages* cow, void* before_page,
                                                      uint64_t offset) {
  if (!cow) {
    return ZX_ERR_INVALID_ARGS;
  }
  return cow->ReplacePageWithLoaned(reinterpret_cast<vm_page_t*>(before_page), offset);
}

void* cpp_vm_cow_pages_get_ref_counted(const VmCowPages* cow) {
  if (!cow) {
    return nullptr;
  }
  return const_cast<fbl::RefCountedUpgradeable<VmCowPages>*>(
      static_cast<const fbl::RefCountedUpgradeable<VmCowPages>*>(cow));
}

void cpp_vm_cow_pages_free(VmCowPages* cow) { delete cow; }

void cpp_vm_cow_pages_initialize_page_cache(uint32_t level) {
  VmCowPages::InitializePageCache(level);
}

}  // extern "C"
