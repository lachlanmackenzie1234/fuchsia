// Copyright 2023 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#ifndef ZIRCON_KERNEL_ARCH_RISCV64_INCLUDE_ARCH_RISCV64_FEATURE_H_
#define ZIRCON_KERNEL_ARCH_RISCV64_INCLUDE_ARCH_RISCV64_FEATURE_H_

#include <stdbool.h>
#include <stdint.h>
#include <zircon/compiler.h>

#include <kernel/ffi.h>

__BEGIN_CDECLS

bool rust_riscv64_feature_has_vector();
uint32_t rust_riscv64_feature_cbom_size();

__END_CDECLS

#ifdef __cplusplus

inline bool riscv64_feature_has_vector() { return rust_riscv64_feature_has_vector(); }

#endif  // __cplusplus

#endif  // ZIRCON_KERNEL_ARCH_RISCV64_INCLUDE_ARCH_RISCV64_FEATURE_H_
