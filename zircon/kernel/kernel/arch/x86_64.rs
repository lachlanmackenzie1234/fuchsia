// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

/// Architecture-specific saved normal mode state for x86_64.
///
/// Saves the normal mode `fs_base` and `gs_base` MSR values across restricted mode entry.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ArchSavedNormalState {
    pub normal_fs_base: u64,
    pub normal_gs_base: u64,
}

const _: () = {
    assert!(core::mem::size_of::<ArchSavedNormalState>() == 16);
    assert!(core::mem::align_of::<ArchSavedNormalState>() == 8);
};
