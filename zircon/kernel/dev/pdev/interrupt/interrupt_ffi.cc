// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT
//
// Ported from zircon/kernel/dev/pdev/interrupt/interrupt.cc

#include <lib/arch/intrin.h>
#include <lib/fit/function.h>
#include <zircon/errors.h>
#include <zircon/types.h>

#include <kernel/ffi.h>
#include <kernel/spinlock.h>
#include <lk/init.h>
#include <pdev/interrupt.h>

#include <ktl/enforce.h>

extern "C" {

struct RustInterruptHandler {
  alignas(16) char data[32];
};

// TODO(https://fxbug.dev/537458631): Remove the annotations once cross-language inlining works.
FFI_ALWAYS_INLINE void cpp_interrupt_handler_assign(RustInterruptHandler* dest, void* src,
                                                    bool is_initialized);
FFI_ALWAYS_INLINE void cpp_interrupt_handler_invoke(const RustInterruptHandler* handler);
FFI_ALWAYS_INLINE bool cpp_interrupt_handler_is_valid(const void* handler);
FFI_ALWAYS_INLINE void cpp_pdev_ops_msi_register_handler(const pdev_interrupt_ops* ops,
                                                         const msi_block_t* block, uint msi_id,
                                                         void* handler);

FFI_ALWAYS_INLINE void cpp_interrupt_handler_assign(RustInterruptHandler* dest, void* src,
                                                    bool is_initialized) {
  auto* dest_handler = reinterpret_cast<interrupt_handler_t*>(dest);
  auto* src_handler = static_cast<interrupt_handler_t*>(src);
  if (is_initialized) {
    *dest_handler = ktl::move(*src_handler);
  } else {
    new (dest_handler) interrupt_handler_t(ktl::move(*src_handler));
  }
}

FFI_ALWAYS_INLINE void cpp_interrupt_handler_invoke(const RustInterruptHandler* handler) {
  (*reinterpret_cast<const interrupt_handler_t*>(handler))();
}

FFI_ALWAYS_INLINE bool cpp_interrupt_handler_is_valid(const void* handler) {
  return static_cast<bool>(*static_cast<const interrupt_handler_t*>(handler));
}

FFI_ALWAYS_INLINE void cpp_pdev_ops_msi_register_handler(const pdev_interrupt_ops* ops,
                                                         const msi_block_t* block, uint msi_id,
                                                         void* handler) {
  ops->msi_register_handler(block, msi_id, ktl::move(*static_cast<interrupt_handler_t*>(handler)));
}

zx_status_t rust_register_int_handler_shim(uint32_t vector, void* handler_ptr, bool permanent);
void rust_msi_register_handler(const msi_block_t* block, uint msi_id, void* handler);

}  // extern "C"

zx_status_t register_int_handler(interrupt_vector_t vector, interrupt_handler_t handler) {
  return rust_register_int_handler_shim(vector, &handler, false);
}

zx_status_t register_permanent_int_handler(interrupt_vector_t vector, interrupt_handler_t handler) {
  return rust_register_int_handler_shim(vector, &handler, true);
}

void msi_register_handler(const msi_block_t* block, uint msi_id, interrupt_handler_t handler) {
  rust_msi_register_handler(block, msi_id, &handler);
}

namespace {

void interrupt_init_percpu_early_hook(uint level) { interrupt_init_percpu_early(); }

LK_INIT_HOOK_FLAGS(interrupt_init_percpu_early, interrupt_init_percpu_early_hook,
                   LK_INIT_LEVEL_PLATFORM_EARLY, LK_INIT_FLAG_SECONDARY_CPUS)

}  // namespace

static_assert(sizeof(interrupt_handler_t) == sizeof(RustInterruptHandler),
              "RustInterruptHandler size mismatch");
static_assert(alignof(interrupt_handler_t) == alignof(RustInterruptHandler),
              "RustInterruptHandler alignment mismatch");

static_assert(sizeof(msi_block_t) == 24, "msi_block_t size mismatch");
static_assert(alignof(msi_block_t) == 8, "msi_block_t alignment mismatch");
