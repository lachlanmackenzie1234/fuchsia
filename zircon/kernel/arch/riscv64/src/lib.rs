// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#![no_std]

unsafe extern "C" {
    fn cpp_riscv64_curr_hart_id() -> u32;
    fn cpp_riscv64_boot_hart_id() -> u32;
}

/// Returns the HART ID of the currently executing hardware thread.
#[inline(always)]
pub fn curr_hart_id() -> u32 {
    // SAFETY: cpp_riscv64_curr_hart_id is a read-only FFI call returning the current HART ID with no side effects.
    unsafe { cpp_riscv64_curr_hart_id() }
}

/// Returns the HART ID of the boot hardware thread.
#[inline(always)]
pub fn boot_hart_id() -> u32 {
    // SAFETY: cpp_riscv64_boot_hart_id is a read-only FFI call returning the boot HART ID with no side effects.
    unsafe { cpp_riscv64_boot_hart_id() }
}
