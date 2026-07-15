// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

/// Architecture-specific saved normal mode state for aarch64.
///
/// Saves the normal mode `tpidr_el0` and `tpidrro_el0` system registers across restricted entry.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ArchSavedNormalState {
    pub tpidr_el0: u64,
    pub tpidrro_el0: u64,
}

const _: () = {
    assert!(core::mem::size_of::<ArchSavedNormalState>() == 16);
    assert!(core::mem::align_of::<ArchSavedNormalState>() == 8);
};
