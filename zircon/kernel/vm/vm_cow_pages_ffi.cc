// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include "vm/vm_cow_pages_ffi.h"

#include <zircon/types.h>

#include "vm/vm_cow_pages.h"

extern "C" {

void* cpp_vm_cow_pages_get_ref_counted(const VmCowPages* cow) {
  if (!cow) {
    return nullptr;
  }
  return const_cast<fbl::RefCountedUpgradeable<VmCowPages>*>(
      static_cast<const fbl::RefCountedUpgradeable<VmCowPages>*>(cow));
}

void cpp_vm_cow_pages_free(VmCowPages* cow) { delete cow; }

}  // extern "C"
