// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use pmm_bindings as bindings;

// Flags for PMM allocation routines.
pub const ALLOC_FLAG_ANY: u32 = bindings::PMM_ALLOC_FLAG_ANY;
pub const ALLOC_FLAG_CAN_WAIT: u32 = bindings::PMM_ALLOC_FLAG_CAN_WAIT;
