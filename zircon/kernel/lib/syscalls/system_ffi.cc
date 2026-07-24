// Copyright 2017 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include <align.h>
#include <debug.h>
#include <lib/arch/asm.h>
#include <lib/boot-options/boot-options.h>
#include <lib/debuglog.h>
#include <lib/fit/defer.h>
#include <lib/instrumentation/asan.h>
#include <lib/page/size.h>
#include <lib/syscalls/forward.h>
#include <lib/zbi-format/kernel.h>
#include <lib/zbi-format/zbi.h>
#include <lib/zbitl/checking.h>
#include <lib/zbitl/view.h>
#include <lib/zircon-internal/macros.h>
#include <platform.h>
#include <string.h>
#include <sys/types.h>
#include <trace.h>
#include <zircon/boot/crash-reason.h>
#include <zircon/compiler.h>
#include <zircon/errors.h>
#include <zircon/rights.h>
#include <zircon/status.h>
#include <zircon/syscalls/resource.h>
#include <zircon/time.h>
#include <zircon/types.h>

#include <cstddef>
#include <cstdint>
#include <cstdio>

#include <arch/arch_ops.h>
#include <arch/mp.h>
#include <arch/ops.h>
#include <dev/hw_watchdog.h>
#include <dev/interrupt.h>
#include <fbl/alloc_checker.h>
#include <fbl/ref_ptr.h>
#include <kernel/cpu.h>
#include <kernel/mp.h>
#include <kernel/percpu.h>
#include <kernel/range_check.h>
#include <kernel/thread.h>
#include <ktl/byte.h>
#include <ktl/span.h>
#include <ktl/unique_ptr.h>
#include <object/process_dispatcher.h>
#include <object/resource.h>
#include <object/vm_object_dispatcher.h>
#include <phys/handoff.h>
#include <platform/halt_helper.h>
#include <platform/mexec.h>
#include <vm/handoff-end.h>
#include <vm/physmap.h>
#include <vm/pmm.h>
#include <vm/vm.h>
#include <vm/vm_aspace.h>

#include "system_priv.h"

#include <ktl/enforce.h>

#define LOCAL_TRACE 0

extern "C" zx_status_t cpp_system_mexec_payload_get_helper(uint8_t* buffer, size_t buffer_size,
                                                           size_t* out_zbi_size);

extern "C" NO_ASAN zx_status_t cpp_system_mexec_core(zx_handle_t resource, zx_handle_t kernel_vmo,
                                                     zx_handle_t data_zbi_vmo);

extern "C" zx_status_t cpp_system_mexec_payload_get_helper(uint8_t* buffer, size_t buffer_size,
                                                           size_t* out_zbi_size) {
  if (auto result = WriteMexecData({reinterpret_cast<ktl::byte*>(buffer), buffer_size});
      result.is_error()) {
    return result.error_value();
  } else {
    *out_zbi_size = ktl::move(result).value();
    return ZX_OK;
  }
}

extern "C" NO_ASAN zx_status_t cpp_system_mexec_core(zx_handle_t resource, zx_handle_t kernel_vmo,
                                                     zx_handle_t data_zbi_vmo) {
  return system_mexec_core(resource, kernel_vmo, data_zbi_vmo);
}
