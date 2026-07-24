// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include <zircon/types.h>

#include <object/vm_address_region_dispatcher.h>

extern "C" {
zx_status_t cpp_vmar_dispatcher_set_memory_priority(VmAddressRegionDispatcher* vmar,
                                                    uint32_t priority);

zx_status_t cpp_vmar_dispatcher_set_memory_priority(VmAddressRegionDispatcher* vmar,
                                                    uint32_t priority) {
  return vmar->SetMemoryPriority(static_cast<VmAddressRegion::MemoryPriority>(priority));
}

}  // extern "C"
