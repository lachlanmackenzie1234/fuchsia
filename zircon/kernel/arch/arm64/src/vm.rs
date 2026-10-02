// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use super::{KERNEL_ASPACE_BASE, KERNEL_ASPACE_SIZE};
use arch_arm64_vm_bindings as vm_bindings;

pub(crate) const USER_BIT_MASK: usize = 0x0080_0000_0000_0000;
zr::static_assert!(USER_BIT_MASK == vm_bindings::kUserBitMask as usize);

/// Check if a virtual address is accessible to user space on aarch64.
///
/// [arm/v8]: D5.2.6 Virtual address splits / TTBR0_EL1 selection bit (VA[55] == 0).
#[inline]
pub fn is_user_accessible(va: usize) -> bool {
    // This address refers to userspace if bit 55 is zero.
    (va & USER_BIT_MASK) == 0
}

/// Returns whether `va` is within the kernel address space.
#[inline]
pub fn is_kernel_address(va: usize) -> bool {
    va >= KERNEL_ASPACE_BASE && va.wrapping_sub(KERNEL_ASPACE_BASE) < KERNEL_ASPACE_SIZE
}

/// Userspace threads can only set an entry point to userspace addresses, or
/// the null pointer (for testing a thread that will always fail).
#[inline]
pub fn is_valid_user_pc(pc: usize) -> bool {
    (pc == 0) || (is_user_accessible(pc) && !is_kernel_address(pc))
}
