// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include <platform.h>
#include <zircon/boot/crash-reason.h>

extern "C" {

void cpp_platform_halt(uint32_t action, uint32_t reason);

void cpp_platform_halt(uint32_t action, uint32_t reason) {
  platform_halt(static_cast<platform_halt_action>(action),
                static_cast<zircon_crash_reason_t>(reason));
}

}  // extern "C"
