// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

/// Architecture-specific saved normal mode state for riscv64.
///
/// Currently riscv64 does not need to save any normal mode state across restricted entry.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ArchSavedNormalState {
    _dummy: u8,
}

const _: () = {
    assert!(core::mem::size_of::<ArchSavedNormalState>() == 1);
    assert!(core::mem::align_of::<ArchSavedNormalState>() == 1);
};
