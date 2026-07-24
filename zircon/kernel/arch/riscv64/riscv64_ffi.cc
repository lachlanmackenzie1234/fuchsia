// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include <arch/riscv64/mp.h>
#include <kernel/ffi.h>

extern "C" {

uint32_t cpp_riscv64_curr_hart_id();
uint32_t cpp_riscv64_boot_hart_id();

FFI_ALWAYS_INLINE uint32_t cpp_riscv64_curr_hart_id() { return riscv64_curr_hart_id(); }
FFI_ALWAYS_INLINE uint32_t cpp_riscv64_boot_hart_id() { return riscv64_boot_hart_id(); }

}  // extern "C"
