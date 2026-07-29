// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include <object/event_dispatcher.h>

extern "C" {

zx_status_t cpp_event_dispatcher_create(uint32_t options,
                                        KernelHandle<EventDispatcher>* handle_out) {
  fbl::AllocChecker ac;
  auto disp = fbl::AdoptRef(new (&ac) EventDispatcher(options));
  if (!ac.check()) {
    return ZX_ERR_NO_MEMORY;
  }

  new (handle_out) KernelHandle<EventDispatcher>(ktl::move(disp));
  return ZX_OK;
}

}  // extern "C"
