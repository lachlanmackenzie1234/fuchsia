// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include "vm/page_queues_ffi.h"

#include <zircon/types.h>

#include <kernel/ffi.h>

#include "vm/page_queues.h"
#include "vm/pmm.h"

// TODO(https://fxbug.dev/537458631): Remove the annotations once cross-language inlining works.
extern "C" {

FFI_ALWAYS_INLINE bool cpp_page_queues_debug_page_is_wired(const PageQueues* queues,
                                                           const vm_page_t* page) {
  return queues->DebugPageIsWired(page);
}

FFI_ALWAYS_INLINE bool cpp_page_queues_debug_page_is_any_anonymous(const PageQueues* queues,
                                                                   const vm_page_t* page) {
  return queues->DebugPageIsAnyAnonymous(page);
}

}  // extern "C"
