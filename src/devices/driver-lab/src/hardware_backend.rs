// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! Hardware access abstraction for the executor.
//!
//! The real driver implements [`MmioBackend`] over a locally mapped MMIO
//! region; tests and driver realm fakes use [`FakeMmio`]. Keeping the trait
//! in the core crate lets the entire read path run on the build host.

use std::collections::{BTreeMap, BTreeSet};

/// Errors surfaced by a hardware backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendError {
    /// The backend could not complete the access; the proxy treats this as
    /// fatal backend state for the current operation.
    Fault,
}

/// A source of target monotonic time, injected so core logic is
/// deterministic under test. The driver supplies zx monotonic time.
pub trait Clock {
    /// Current timestamp in nanoseconds.
    fn now_ns(&mut self) -> i64;
}

/// Deterministic test clock advancing by a fixed step per reading.
#[derive(Clone, Copy, Debug)]
pub struct FakeClock {
    /// Timestamp returned by the next call.
    pub now: i64,
    /// Amount added after each call.
    pub step: i64,
}

impl Clock for FakeClock {
    fn now_ns(&mut self) -> i64 {
        let now = self.now;
        self.now += self.step;
        now
    }
}

/// Volatile 32-bit MMIO access to one resource.
pub trait MmioBackend {
    /// Performs one volatile 32-bit read at `offset` bytes from the start
    /// of the logical resource.
    fn read32(&mut self, offset: u64) -> Result<u32, BackendError>;
}

/// In-memory fake MMIO region that records every access, for unit tests
/// and driver realm fakes. Reads of unset offsets return zero.
#[derive(Clone, Debug, Default)]
pub struct FakeMmio {
    values: BTreeMap<u64, u32>,
    faults: BTreeSet<u64>,
    /// Offsets read, in order.
    pub accesses: Vec<u64>,
}

impl FakeMmio {
    /// Creates an empty fake region.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the value returned for reads at `offset`.
    pub fn set(&mut self, offset: u64, value: u32) {
        self.values.insert(offset, value);
    }

    /// Makes reads at `offset` fail with [`BackendError::Fault`].
    pub fn fail_at(&mut self, offset: u64) {
        self.faults.insert(offset);
    }
}

impl MmioBackend for FakeMmio {
    fn read32(&mut self, offset: u64) -> Result<u32, BackendError> {
        self.accesses.push(offset);
        if self.faults.contains(&offset) {
            return Err(BackendError::Fault);
        }
        Ok(self.values.get(&offset).copied().unwrap_or(0))
    }
}
