// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

/// An enumeration of various synchronization options to use when performing
/// memory transfer operations (such as to and from a sequence lock's payload or
/// during well-defined copy operations).
///
/// # Options
///
/// * **AcqRelOps**: Use either `Ordering::Acquire` (reading the payload / `CopyFrom`)
///   or `Ordering::Release` (writing the payload / `CopyTo`) on every atomic load/store
///   operation during the transfer to/from the shared buffer.
/// * **Fence**: Use either an `Ordering::Acquire` thread fence (reading the payload /
///   `CopyFrom`) after the transfer operation, or an `Ordering::Release` thread
///   fence (writing the payload / `CopyTo`) before the operation, and `Ordering::Relaxed`
///   for each of the atomic load/store operations during the transfer.
/// * **None**: Simply use `Ordering::Relaxed` for each of the atomic load/store
///   operations during the transfer. Do not actually introduce any explicit
///   synchronization behavior.
///
/// WARNING: Use cases for the `None` transfer mode tend to be unusual. Users
/// will almost always want some form of synchronization to take place during
/// their transfers. One example of where it may be appropriate to use
/// `SyncOpt::None` might be a situation where users are attempting to observe
/// the state of more than one object while inside of a sequence lock read
/// transaction, and the user has decided that it is better to use a thread
/// fence than to use acquire semantics on each element transferred. Such a
/// sequence might look something like this:
///
/// ```cpp
/// Foo foo1, foo2;
/// Bar bar1, bar2;
/// ...
/// WellDefinedCopyFrom<SyncOpt::None, alignof(Foo)>(&foo1, &src_foo1, sizeof(foo1));
/// WellDefinedCopyFrom<SyncOpt::None, alignof(Foo)>(&foo2, &src_foo2, sizeof(foo2));
/// WellDefinedCopyFrom<SyncOpt::None, alignof(Foo)>(&bar1, &src_bar1, sizeof(bar1));
/// WellDefinedCopyFrom<SyncOpt::Fence, alignof(Foo)>(&bar2, &src_bar2, sizeof(bar2));
/// ```
///
/// TODO: Translate this example ot Rust once a Rust analog to WellDefinedCopyFrom
/// exists in Rust.
///
/// Note that it is the _last_ transfer operation which includes the fence. In
/// the case of a `CopyTo` operation (when publishing data) it would be the _first_
/// operation which included the fence, not the last.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncOpt {
    AcqRelOps,
    Fence,
    None,
}

impl SyncOpt {
    /// Converts a `u8` discriminant to a [`SyncOpt`].
    ///
    /// # Panics
    ///
    /// Panics if `val` is not a valid [`SyncOpt`] discriminant (`SYNC_OPT_ACQ_REL_OPS`,
    /// `SYNC_OPT_FENCE`, or `SYNC_OPT_NONE`).
    pub const fn from_u8(val: u8) -> Self {
        match val {
            SYNC_OPT_ACQ_REL_OPS => SyncOpt::AcqRelOps,
            SYNC_OPT_FENCE => SyncOpt::Fence,
            SYNC_OPT_NONE => SyncOpt::None,
            _ => panic!("invalid SyncOpt discriminant"),
        }
    }
}

// Const generic parameters cannot be user defined enumerations without the
// unstable `adt_const_params` feature, which the Fuchsia build disallows, so
// `SeqLock` is parameterized by these `SyncOpt` discriminants instead.
pub const SYNC_OPT_ACQ_REL_OPS: u8 = SyncOpt::AcqRelOps as u8;
pub const SYNC_OPT_FENCE: u8 = SyncOpt::Fence as u8;
pub const SYNC_OPT_NONE: u8 = SyncOpt::None as u8;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_u8() {
        assert_eq!(SyncOpt::from_u8(SYNC_OPT_ACQ_REL_OPS), SyncOpt::AcqRelOps);
        assert_eq!(SyncOpt::from_u8(SYNC_OPT_FENCE), SyncOpt::Fence);
        assert_eq!(SyncOpt::from_u8(SYNC_OPT_NONE), SyncOpt::None);
    }

    #[test]
    #[should_panic(expected = "invalid SyncOpt discriminant")]
    fn test_from_u8_invalid() {
        SyncOpt::from_u8(42);
    }
}
