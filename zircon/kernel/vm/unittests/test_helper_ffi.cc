// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include "test_helper_ffi.h"

#include <zircon/types.h>

#include "test_helper.h"

extern "C" {

zx_status_t cpp_make_committed_pager_vmo(size_t num_pages, bool trap_dirty, bool resizable,
                                         void** out_pages, VmObjectPaged** out_vmo) {
  fbl::RefPtr<VmObjectPaged> vmo;
  zx_status_t status = vm_unittest::make_committed_pager_vmo(
      num_pages, trap_dirty, resizable, reinterpret_cast<vm_page_t**>(out_pages), &vmo);
  if (status != ZX_OK) {
    return status;
  }
  if (out_vmo) {
    *out_vmo = fbl::ExportToRawPtr(&vmo);
  }
  return ZX_OK;
}

}  // extern "C"
