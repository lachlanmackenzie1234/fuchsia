// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#![no_std]

unsafe extern "C" {
    fn cpp_boot_options_enable_debugging_syscalls() -> bool;
}

/// Returns whether debugging syscalls are enabled in kernel boot options.
// TODO(https://fxbug.dev/537008680): Replace per-flag FFI query with direct BootOptions struct
// layout binding.
pub fn enable_debugging_syscalls() -> bool {
    // SAFETY: FFI query function reads global boot/feature option and has no side effects.
    unsafe { cpp_boot_options_enable_debugging_syscalls() }
}
