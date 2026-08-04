// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use crate::vm::vm_object::VmObject;
use fbl::RefPtr;
use zx_status::Status;

/// An RAII wrapper around a `VmObject` that is pinned.
///
/// This is an independent Rust type and does not guarantee memory layout compatibility with the
/// C++ `PinnedVmObject` class. Unlike the C++ version, `PinnedVmObject` in Rust cannot represent
/// an empty state; optionality is handled via `Option<PinnedVmObject>`.
pub struct PinnedVmObject {
    vmo: RefPtr<VmObject>,
    offset: u64,
    size: u64,
}

impl PinnedVmObject {
    /// Pins `size` bytes starting at `offset` in `vmo` and returns a
    /// `PinnedVmObject`.
    pub fn create(
        vmo: RefPtr<VmObject>,
        offset: u64,
        size: u64,
        write: bool,
    ) -> Result<Self, Status> {
        debug_assert!(page::is_aligned(offset as usize) && page::is_aligned(size as usize));
        vmo.commit_range_pinned(offset, size, write)?;
        Ok(Self { vmo, offset, size })
    }

    /// Returns a reference to the underlying `VmObject`.
    pub fn vmo(&self) -> &RefPtr<VmObject> {
        &self.vmo
    }

    /// Returns the offset into the VMO where the pinned range starts.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Returns the size of the pinned range.
    pub fn size(&self) -> u64 {
        self.size
    }
}

impl Drop for PinnedVmObject {
    fn drop(&mut self) {
        self.vmo.unpin(self.offset, self.size);
    }
}
