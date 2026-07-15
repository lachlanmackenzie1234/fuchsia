// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

// Normal Rust file-finding works with "test_submod.rs" in the GN sources list.
mod test_submod;

use test_submod::*;

#[unsafe(no_mangle)]
pub extern "C" fn rust_mod_fn() -> i32 {
    test_fn()
}
