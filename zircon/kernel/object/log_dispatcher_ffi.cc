// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include <zircon/rights.h>

#include <fbl/alloc_checker.h>
#include <fbl/ref_ptr.h>
#include <ktl/utility.h>
#include <object/handle.h>
#include <object/log_dispatcher.h>

extern "C" {

zx_status_t cpp_log_dispatcher_create(uint32_t flags, zx_rights_t rights,
                                      KernelHandle<LogDispatcher>* handle_out) {
  fbl::AllocChecker ac;
  auto disp = fbl::AdoptRef(new (&ac) LogDispatcher(flags));
  if (!ac.check()) {
    return ZX_ERR_NO_MEMORY;
  }

  new (handle_out) KernelHandle<LogDispatcher>(ktl::move(disp));
  return ZX_OK;
}

}  // extern "C"
