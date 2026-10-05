// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#ifndef ZIRCON_KERNEL_VM_INCLUDE_VM_EVICTOR_FFI_H_
#define ZIRCON_KERNEL_VM_INCLUDE_VM_EVICTOR_FFI_H_

#include <zircon/compiler.h>

#include <kernel/ffi.h>
#include <vm/evictor.h>

using EvictionLevel = Evictor::EvictionLevel;
using Output = Evictor::Output;
using TriggerReason = Evictor::TriggerReason;
using EvictionResult = Evictor::EvictionResult;
using EvictedPageCounts = Evictor::EvictedPageCounts;

// TODO(https://fxbug.dev/537458631): Remove the annotations once cross-language inlining works.
__BEGIN_CDECLS

FFI_ALWAYS_INLINE void cpp_evictor_disable_eviction(Evictor* evictor);
FFI_ALWAYS_INLINE void cpp_evictor_init(ffi::Uninitialized<Evictor>* evictor);
void cpp_evictor_evict_synchronous(Evictor* evictor, uint64_t min_mem_to_free,
                                   uint64_t free_mem_target, EvictionLevel eviction_level,
                                   Output output, TriggerReason reason,
                                   ffi::Uninitialized<EvictionResult>* out_result);
void cpp_evictor_evict_asynchronous(Evictor* evictor, uint64_t min_free_target,
                                    uint64_t free_mem_target, bool print_output);

__END_CDECLS

#endif  // ZIRCON_KERNEL_VM_INCLUDE_VM_EVICTOR_FFI_H_
