// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

pub mod blob;
pub mod extents;
pub mod pager;
pub mod reader;

#[cfg(test)]
pub mod testing;

pub use blob::{Blob, Blobs};
pub use extents::{Extent, Extents, ExtentsIterator};
pub use pager::{PagerThread, run_pager_loop};

pub const BLOCK_SIZE: u64 = 4096;
