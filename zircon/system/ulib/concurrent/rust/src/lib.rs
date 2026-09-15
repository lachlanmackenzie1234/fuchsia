// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#![no_std]

#[cfg(test)]
extern crate std;

mod common;
mod seqlock;

pub use common::{SYNC_OPT_ACQ_REL_OPS, SYNC_OPT_FENCE, SYNC_OPT_NONE, SyncOpt};
pub use seqlock::{ReadTransactionToken, SeqLock, SequenceNumber, WriteGuard};
