// Copyright 2016 The Fuchsia Authors
// Copyright (c) 2008-2014 Travis Geiselbrecht
// Copyright (c) 2012 Shantanu Gupta
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

//! Kernel mutex support, mirroring the C++ `Mutex` and `CriticalMutex` declared in
//! `<kernel/mutex.h>`.

use core::cell::UnsafeCell;
use core::marker::{PhantomData, PhantomPinned};
use core::mem::{align_of, offset_of, size_of};
use core::sync::atomic::{AtomicPtr, AtomicU32};
use mutex_bindings as bindings;
use pin_init::pin_data;
use zr::static_assert;

use crate::kernel::lazy_owned_wait_queue::LazyOwnedWaitQueue;
use crate::kernel::spin_tracing::MutexNameStorage;
use crate::kernel::thread::Thread;
use crate::kernel::types::cpu_num_t;
use crate::platform_rs::timer::DurationMono;

/// Magic value identifying a [`Mutex`] (`'mutx'`).
const MAGIC: u32 = fbl::magic(b"mutx");
/// The `val` address of an unowned [`Mutex`].
const STATE_FREE: usize = 0;
/// Flag bit in `val` indicating that at least one thread is blocked waiting on the [`Mutex`].
const STATE_FLAG_CONTESTED: usize = 1;

static_assert!(MAGIC == bindings::MutexLayout_kMagic);
static_assert!(STATE_FREE == bindings::MutexLayout_kStateFree);
static_assert!(STATE_FLAG_CONTESTED == bindings::MutexLayout_kStateFlagContested);

/// Kernel mutex support.
///
/// Layout-compatible with the C++ `Mutex` class, including its
/// `spin_tracing::LockNameStorage` base, which is empty unless scheduler lock spin tracing or
/// lock name tracing is enabled.
///
/// Every byte of a `Mutex` is interior-mutable (atomics or `UnsafeCell`, with no padding) so
/// `&Mutex` carries no `Freeze` bytes. Once a releasing thread clears `val` (or detaches and
/// unlocks the contended `OwnedWaitQueue` in `ReleaseContendedMutex`), another CPU may acquire,
/// release, and destroy the `Mutex` (zeroing `magic` in `~Canary`) before the releasing `&Mutex`
/// call frame returns. Do not add a `Freeze` field.
#[pin_data]
#[repr(C)]
pub struct Mutex {
    /// Storage for the lock name and encoded lock ID used by lock tracing. Corresponds to the
    /// C++ `spin_tracing::LockNameStorage` base class, and is zero-sized when
    /// `MUTEX_NAME_TRACING_ENABLED` is false. Wrapped in `UnsafeCell` so `Mutex` has no `Freeze`
    /// bytes.
    name_storage: UnsafeCell<MutexNameStorage>,

    /// Structure guard (`'mutx'` while live, zeroed on drop). Wrapped in `UnsafeCell` so
    /// tearing down on another CPU cannot conflict with an unwinding `&Mutex` borrow.
    magic: UnsafeCell<fbl::Canary<MAGIC>>,

    /// See the comment near the start of C++ `Mutex::AcquireContendedMutex` for details about
    /// this variable, and how it affects the spin phase behavior of
    /// `Mutex::AcquireContendedMutex`.
    maybe_acquired_on_cpu: AtomicU32,
    /// Holds the owning `Thread` pointer bitwise-ORed with [`STATE_FLAG_CONTESTED`], or null
    /// ([`STATE_FREE`]) when unowned. Typed as `AtomicPtr<Thread>` to preserve pointer provenance.
    val: AtomicPtr<Thread>,
    #[pin]
    wait: LazyOwnedWaitQueue,

    /// No moving or copying allowed; wrapped in `PhantomData` so `Mutex` and types embedding it
    /// stay FFI-safe (`improper_ctypes` rejects a bare `PhantomPinned`).
    #[pin]
    _pin: PhantomData<PhantomPinned>,
}

// SAFETY: `Mutex` has no thread-local ownership invariants and is `!Unpin`, so ownership of a
// pinned `Mutex` can be transferred across threads.
unsafe impl Send for Mutex {}
// SAFETY: Shared accesses through `&Mutex` operate on the `maybe_acquired_on_cpu`, `val`, and
// `wait` atomics, or perform volatile reads of `magic` while the mutex is live. `name_storage` is
// only mutated during initialization / single-threaded lock-class setup before concurrent use, and
// `magic` is only mutated during teardown after the mutex is unlocked and any contended
// `OwnedWaitQueue` lock is released; wrapping both in `UnsafeCell` ensures `&Mutex` has no
// `Freeze` bytes if another CPU destroys the mutex before a releasing `&Mutex` call returns.
unsafe impl Sync for Mutex {}

impl Mutex {
    /// The maximum duration to spin before falling back to blocking.
    // TODO(https://fxbug.dev/42109976): Decide how to make this configurable per device/platform
    // and describe how to optimize this value.
    pub const SPIN_MAX_DURATION: DurationMono = DurationMono::from_micros(150);

    /// A constant for users of `AutoExpiringPreemptDisabler` to select a value
    /// consistent with the defaults for [`Mutex`]/[`CriticalMutex`]. Using this constant
    /// also improves readability, providing a link to the motivating use case for
    /// timeslice extension.
    pub const DEFAULT_TIMESLICE_EXTENSION: DurationMono = Self::SPIN_MAX_DURATION;
}

static_assert!(Mutex::SPIN_MAX_DURATION.into_nanos() == bindings::MutexLayout_kSpinMaxDuration);
static_assert!(
    Mutex::DEFAULT_TIMESLICE_EXTENSION.into_nanos()
        == bindings::MutexLayout_kDefaultTimesliceExtension
);

// `maybe_acquired_on_cpu` is a C++ `ktl::atomic<cpu_num_t>`.
static_assert!(size_of::<AtomicU32>() == size_of::<cpu_num_t>());
static_assert!(align_of::<AtomicU32>() == align_of::<cpu_num_t>());

// Compile-time layout assertions against the `MutexLayout` constants and opaque `Mutex` type
// generated by bindgen of `<kernel/mutex_layout.h>`. The C++ `spin_tracing::LockNameStorage` base
// class is always at offset zero.
static_assert!(size_of::<Mutex>() == bindings::MutexLayout_kSize);
static_assert!(align_of::<Mutex>() == bindings::MutexLayout_kAlign);
static_assert!(size_of::<Mutex>() == size_of::<bindings::Mutex>());
static_assert!(align_of::<Mutex>() == align_of::<bindings::Mutex>());
static_assert!(offset_of!(Mutex, name_storage) == 0);
static_assert!(size_of::<UnsafeCell<MutexNameStorage>>() == bindings::MutexLayout_kNameStorageSize);
static_assert!(offset_of!(Mutex, magic) == bindings::MutexLayout_kMagicOffset);
static_assert!(
    offset_of!(Mutex, maybe_acquired_on_cpu) == bindings::MutexLayout_kMaybeAcquiredOnCpuOffset
);
static_assert!(offset_of!(Mutex, val) == bindings::MutexLayout_kValOffset);
static_assert!(offset_of!(Mutex, wait) == bindings::MutexLayout_kWaitOffset);

/// `CriticalMutex` is a mutex variant that uses a thread timeslice extension to
/// disable preemption for a limited time during the critical section.
///
/// Acquiring a `CriticalMutex` sets a timeslice extension that's cleared when the
/// mutex is released or the extension expires, whichever comes first.
///
/// This variant is useful to avoid a form of thrash where a thread holding a
/// mutex is preempted by another thread that subsequently blocks while trying to
/// acquire the already held mutex.  `CriticalMutex` is designed to mitigate this
/// type of thrash by preventing the holder from being preempted, up to some
/// maximum amount of time.  By limiting maximum amount of time the holder is
/// non-preemptible, we can limit the degree to which other, logically higher
/// priority tasks are delayed from executing.
///
/// Good candidates for `CriticalMutex` are global or widely shared locks that
/// typically, but not necessarily always, have very short critical sections
/// (tens of microseconds or less) and high contention under load.
///
/// `CriticalMutex` differs from `SpinLock` in the following ways:
/// * Threads contending a `CriticalMutex` will block after the spin interval is
///   exceeded, avoiding extended monopolization of multiple CPUs.
/// * Threads may block while holding a `CriticalMutex`, simplifying maintaining
///   invariants in slow paths.
/// * Interrupts may remain enabled while holding a `CriticalMutex`, avoiding
///   undesirable IRQ latency.
///
/// Layout-compatible with the C++ `CriticalMutex` class, which privately inherits from `Mutex`
/// and adds no fields of its own.
#[pin_data]
#[repr(transparent)]
pub struct CriticalMutex {
    /// The underlying [`Mutex`]. Corresponds to the C++ private `Mutex` base class.
    #[pin]
    mutex: Mutex,
}

// Compile-time layout assertions against the `MutexLayout` constants and opaque `CriticalMutex`
// type generated by bindgen of `<kernel/mutex_layout.h>`.
static_assert!(size_of::<CriticalMutex>() == bindings::MutexLayout_kCriticalMutexSize);
static_assert!(align_of::<CriticalMutex>() == bindings::MutexLayout_kCriticalMutexAlign);
static_assert!(size_of::<CriticalMutex>() == size_of::<bindings::CriticalMutex>());
static_assert!(align_of::<CriticalMutex>() == align_of::<bindings::CriticalMutex>());
// The C++ private `Mutex` base class is CriticalMutex's only base and is at offset zero.
static_assert!(offset_of!(CriticalMutex, mutex) == 0);
