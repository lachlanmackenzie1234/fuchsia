// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include "vm/physical_page_borrowing_config_ffi.h"

#include "vm/physical_page_borrowing_config.h"

extern "C" {

bool cpp_set_loaning_enabled(bool enabled) {
  bool prev = PhysicalPageBorrowingConfig::Get().is_loaning_enabled();
  PhysicalPageBorrowingConfig::Get().set_loaning_enabled(enabled);
  return prev;
}

}  // extern "C"
