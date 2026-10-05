// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include "vm/evictor_ffi.h"

#include <kernel/ffi.h>
#include <ktl/memory.h>

// TODO(https://fxbug.dev/537458631): Remove the annotations once cross-language inlining works.
extern "C" {

FFI_ALWAYS_INLINE void cpp_evictor_disable_eviction(Evictor* evictor) {
  evictor->DisableEviction();
}
FFI_ALWAYS_INLINE void cpp_evictor_init(ffi::Uninitialized<Evictor>* evictor) {
  evictor->Initialize();
}

void cpp_evictor_evict_synchronous(Evictor* evictor, uint64_t min_mem_to_free,
                                   uint64_t free_mem_target, EvictionLevel eviction_level,
                                   Output output, TriggerReason reason,
                                   ffi::Uninitialized<EvictionResult>* out_result) {
  out_result->Initialize(
      evictor->EvictSynchronous(min_mem_to_free, free_mem_target, eviction_level, output, reason));
}

void cpp_evictor_evict_asynchronous(Evictor* evictor, uint64_t min_free_target,
                                    uint64_t free_mem_target, bool print_output) {
  evictor->EvictAsynchronous(min_free_target, free_mem_target, EvictionLevel::OnlyOldest,
                             print_output ? Output::Print : Output::NoPrint);
}

}  // extern "C"
