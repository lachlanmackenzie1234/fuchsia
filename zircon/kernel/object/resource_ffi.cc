// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include <object/resource.h>

extern "C" {

zx_status_t cpp_resource_validate_resource_kind_base(zx_handle_t handle, zx_rsrc_kind_t kind,
                                                     zx_rsrc_system_base_t base) {
  return validate_resource_kind_base(handle, kind, base);
}

zx_status_t cpp_resource_validate_ranged_resource(zx_handle_t handle, zx_rsrc_kind_t kind,
                                                  uintptr_t base, size_t size) {
  return validate_ranged_resource(handle, kind, base, size);
}

}  // extern "C"
