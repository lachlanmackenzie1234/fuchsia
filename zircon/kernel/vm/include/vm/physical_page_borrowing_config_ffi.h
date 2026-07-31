// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#ifndef ZIRCON_KERNEL_VM_INCLUDE_VM_PHYSICAL_PAGE_BORROWING_CONFIG_FFI_H_
#define ZIRCON_KERNEL_VM_INCLUDE_VM_PHYSICAL_PAGE_BORROWING_CONFIG_FFI_H_

#include <stdbool.h>
#include <zircon/compiler.h>

#include <vm/physical_page_borrowing_config.h>

__BEGIN_CDECLS

bool cpp_set_loaning_enabled(bool enabled);

__END_CDECLS

#endif  // ZIRCON_KERNEL_VM_INCLUDE_VM_PHYSICAL_PAGE_BORROWING_CONFIG_FFI_H_
