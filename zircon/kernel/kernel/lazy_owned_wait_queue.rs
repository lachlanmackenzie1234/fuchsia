// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

//! Rust layout of the C++ `LazyOwnedWaitQueue` declared in `<kernel/lazy_owned_wait_queue.h>`.

use core::ffi::c_void;
use core::marker::{PhantomData, PhantomPinned};
use core::mem::{align_of, offset_of, size_of};
use core::sync::atomic::AtomicPtr;
use pin_init::pin_data;
use zr::static_assert;

use crate::kernel::owned_wait_queue::OwnedWaitQueue;

/// A wrapper class for [`OwnedWaitQueue`] that lazily borrows a queue from a pool
/// when necessary.
///
/// Layout-compatible with the C++ `LazyOwnedWaitQueue`
/// (`LazyOwnedWaitQueueImpl<GlobalOwqPoolAccess>`), whose `GlobalOwqPoolAccess` base is empty.
#[pin_data]
#[repr(C)]
pub struct LazyOwnedWaitQueue {
    ptr: AtomicPtr<OwnedWaitQueue>,
    /// No moving or copying allowed; wrapped in `PhantomData` so types embedding
    /// `LazyOwnedWaitQueue` stay FFI-safe (`improper_ctypes` rejects a bare `PhantomPinned`).
    #[pin]
    _pin: PhantomData<PhantomPinned>,
}

// LazyOwnedWaitQueue must be exactly pointer-sized.
static_assert!(size_of::<LazyOwnedWaitQueue>() == size_of::<*mut c_void>());
static_assert!(align_of::<LazyOwnedWaitQueue>() == align_of::<*mut c_void>());
static_assert!(offset_of!(LazyOwnedWaitQueue, ptr) == 0);
