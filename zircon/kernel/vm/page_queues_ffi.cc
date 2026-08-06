// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include "vm/page_queues_ffi.h"

#include <zircon/types.h>

#include "vm/page_queues.h"
#include "vm/pmm.h"

extern "C" {

bool cpp_page_queues_debug_page_is_wired(const PageQueues* queues, const void* page) {
  return queues->DebugPageIsWired(reinterpret_cast<const vm_page_t*>(page));
}

bool cpp_page_queues_debug_page_is_any_anonymous(const PageQueues* queues, const void* page) {
  return queues->DebugPageIsAnyAnonymous(reinterpret_cast<const vm_page_t*>(page));
}

}  // extern "C"
