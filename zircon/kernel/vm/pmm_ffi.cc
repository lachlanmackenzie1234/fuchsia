// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include "vm/pmm_ffi.h"

#include <zircon/types.h>

#include "vm/pmm.h"

extern "C" {

void* cpp_paddr_to_vm_page(zx_paddr_t paddr) { return paddr_to_vm_page(paddr); }

PageQueues* cpp_pmm_page_queues() { return pmm_page_queues(); }

zx_status_t cpp_pmm_alloc_page(uint32_t flags, void** out_page, zx_paddr_t* out_paddr) {
  vm_page_t* page = nullptr;
  paddr_t paddr = 0;
  zx_status_t status = pmm_alloc_page(flags, &page, &paddr);
  if (status == ZX_OK) {
    if (out_page) {
      *out_page = page;
    }
    if (out_paddr) {
      *out_paddr = paddr;
    }
  }
  return status;
}

void cpp_pmm_free_page(void* page) { pmm_free_page(reinterpret_cast<vm_page_t*>(page)); }

}  // extern "C"
