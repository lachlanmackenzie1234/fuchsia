// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use core::ffi::c_void;
use ksync::{
    ConstRawLock, ExclusiveLockPolicy, LockPolicy, LockdepTracked, RawAssertHeldLock, RawLock,
    RawUntrackedIrqLock, RawUntrackedLock, impl_lockdep_tracked_lock_policy,
};
use pin_init::{PinInit, pin_data};

#[cfg(target_arch = "x86_64")]
type InnerSavedState = u64;
#[cfg(not(target_arch = "x86_64"))]
type InnerSavedState = bool;

/// Opaque token representing the saved interrupt state.
#[must_use = "saved interrupt state must be passed to a matching `release_irq_restore`"]
#[repr(transparent)]
#[derive(Copy, Clone, Default)]
pub struct InterruptSavedState(InnerSavedState);

/// Outcome of a non-blocking [`RawSpinlockStorage::try_acquire`] attempt.
#[must_use = "if `Acquired`, the spinlock is now held and must be released"]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TryAcquireResult {
    /// The spinlock was acquired by the calling CPU and must be released.
    Acquired,
    /// The spinlock was already held and was not acquired.
    Failed,
}

impl TryAcquireResult {
    /// Returns `true` if the spinlock was acquired (and so must be released).
    #[inline]
    pub const fn is_acquired(self) -> bool {
        matches!(self, Self::Acquired)
    }
}

unsafe extern "C" {
    fn cpp_ksync_interrupt_save() -> InterruptSavedState;

    fn cpp_spinlock_init(lock: *mut c_void, class_id: *const c_void);
    fn cpp_spinlock_destroy(lock: *mut c_void);
    fn cpp_spinlock_acquire_irqsave(lock: *mut c_void) -> InterruptSavedState;
    fn cpp_spinlock_release_irqrestore(lock: *mut c_void, state: InterruptSavedState);
    fn cpp_spinlock_acquire_no_irqsave(lock: *mut c_void);
    fn cpp_spinlock_try_acquire(lock: *mut c_void) -> bool;
    fn cpp_spinlock_release_no_irqrestore(lock: *mut c_void);
    fn cpp_spinlock_is_held(lock: *const c_void) -> bool;
    fn cpp_spinlock_assert_held(lock: *const c_void);

    fn cpp_monitored_spinlock_init(lock: *mut c_void, class_id: *const c_void);
    fn cpp_monitored_spinlock_destroy(lock: *mut c_void);
    fn cpp_monitored_spinlock_acquire_irqsave(
        lock: *mut c_void,
        name: *const core::ffi::c_char,
    ) -> InterruptSavedState;
    fn cpp_monitored_spinlock_release_irqrestore(lock: *mut c_void, state: InterruptSavedState);
    fn cpp_monitored_spinlock_acquire_no_irqsave(lock: *mut c_void, name: *const core::ffi::c_char);
    fn cpp_monitored_spinlock_release_no_irqrestore(lock: *mut c_void);
}

#[cfg(scheduler_lock_spin_tracing_enabled)]
const RAW_SPINLOCK_SIZE: usize = 16;
#[cfg(not(scheduler_lock_spin_tracing_enabled))]
const RAW_SPINLOCK_SIZE: usize = 4;

/// Opaque layout block for the Zircon C++ `SpinLock` object.
///
/// Note: When `spin_lock_tracing` is disabled, C++ `SpinLock` has size 4 and alignment 4, whereas
/// this type intentionally over-aligns to 8 bytes (`#[repr(C, align(8))]`, resulting in size 8 and
/// alignment 8) so that `RawSpinlock` and `#[repr(C)]` FFI structs embedding it (such as
/// `ControlRegion` and `TrapMap`, which pad `sizeof(LockType)` to 8 bytes in C++) maintain uniform
/// 8-byte alignment across all `lock_dep` and `spin_lock_tracing` configurations.
#[pin_data(PinnedDrop)]
#[repr(C, align(8))]
pub struct RawSpinlockStorage {
    storage: zr::OpaqueBytes<RAW_SPINLOCK_SIZE>,
}

impl RawSpinlockStorage {
    #[allow(clippy::declare_interior_mutable_const)]
    pub const INIT: Self = Self::const_init();

    /// Statically initializes a `RawSpinlockStorage` in constant context.
    pub const fn const_init() -> Self {
        Self { storage: zr::OpaqueBytes::new([0u8; RAW_SPINLOCK_SIZE]) }
    }

    /// Initializes a `RawSpinlockStorage` in place.
    #[inline]
    pub fn new() -> impl PinInit<Self, core::convert::Infallible> {
        // SAFETY: Passing a null `class_id` is always valid for an untracked spinlock.
        unsafe { <Self as RawLock>::init(core::ptr::null()) }
    }

    /// Acquire the spinlock.
    ///
    /// Interrupts must already be disabled.
    ///
    /// # Safety
    ///
    /// Interrupts must already be disabled on the calling CPU, the calling CPU must not already
    /// hold this spinlock, and this acquisition must be paired with a matching [`Self::release`]
    /// or [`Self::release_irq_restore`] on the same CPU before interrupts are re-enabled or the
    /// thread blocks or migrates.
    #[inline]
    pub unsafe fn acquire(&self) {
        // SAFETY: `self.storage.get()` points to an initialized `SpinLock` with write provenance,
        // and the caller upholds the interrupt-disabled and non-reentrant locking preconditions.
        unsafe {
            cpp_spinlock_acquire_no_irqsave(self.storage.get().cast::<c_void>());
        }
    }

    /// Attempt to acquire the spinlock without waiting.
    ///
    /// Interrupts must already be disabled.
    ///
    /// Returns [`TryAcquireResult::Acquired`] when the lock is acquired, and
    /// [`TryAcquireResult::Failed`] when the lock is not acquired.
    ///
    /// `try_acquire` operations are not permitted to fail spuriously, even on architectures with
    /// weak memory ordering. If a `try_acquire` operation fails, it must be because the lock was
    /// actually observed to be held during the attempt.
    ///
    /// # Safety
    ///
    /// Interrupts must already be disabled on the calling CPU, and if this call acquires the lock
    /// (returns [`TryAcquireResult::Acquired`]), it must be paired with a matching
    /// [`Self::release`] or [`Self::release_irq_restore`] on the same CPU before interrupts are
    /// re-enabled or the thread blocks or migrates.
    #[inline]
    pub unsafe fn try_acquire(&self) -> TryAcquireResult {
        // SAFETY: `self.storage.get()` points to an initialized `SpinLock` with write provenance,
        // and the caller upholds the interrupt-disabled locking preconditions.
        // C++ `SpinLock::TryAcquire()` returns `false` when acquired and `true` when not acquired.
        if unsafe { cpp_spinlock_try_acquire(self.storage.get().cast::<c_void>()) } {
            TryAcquireResult::Failed
        } else {
            TryAcquireResult::Acquired
        }
    }

    /// Release the spinlock.
    ///
    /// Interrupts must already be disabled.
    ///
    /// # Safety
    ///
    /// Interrupts must be disabled and this spinlock must currently be held by the calling CPU
    /// without outstanding references protected by this acquisition.
    ///
    /// The allocation backing `self` must remain live until this call returns. Once the lock word
    /// is released another CPU may acquire the lock immediately, so this must not be used to
    /// release a lock that guards the destruction of its own containing allocation.
    #[inline]
    pub unsafe fn release(&self) {
        // SAFETY: `self.storage.get()` points to an initialized `SpinLock` with write provenance,
        // and the caller guarantees that the calling CPU currently holds the spinlock with
        // interrupts disabled.
        unsafe {
            cpp_spinlock_release_no_irqrestore(self.storage.get().cast::<c_void>());
        }
    }

    /// Returns `true` if held by the calling CPU.
    ///
    /// Interrupts must be disabled before calling this method, otherwise it could return `true`
    /// when it should return `false`.
    ///
    /// # Panics
    ///
    /// In builds with kernel debug assertions enabled, panics if interrupts are not disabled on
    /// the calling CPU. Without debug assertions no check is made, and calling this with
    /// interrupts enabled (and so migration possible) can return a false positive.
    #[inline]
    pub fn is_held(&self) -> bool {
        // SAFETY: `self.storage.get()` points to an initialized `SpinLock`. C++
        // `SpinLock::IsHeld()` debug-asserts `arch_ints_disabled()` and then calls
        // `arch_spin_lock_held()`, which performs a relaxed atomic load of the lock word and
        // compares it against `arch_curr_cpu_num()`, which reads the architectural per-CPU
        // register state (e.g. `%gs`, `x20`, or `tp`) that is set up before any spinlock can be
        // used. Both are memory-safe in all states.
        unsafe { cpp_spinlock_is_held(self.storage.get().cast_const().cast::<c_void>()) }
    }

    /// Just like [`Self::acquire`], but saves interrupt state and disables interrupts.
    ///
    /// # Safety
    ///
    /// The calling CPU must not already hold this spinlock, and the returned
    /// [`InterruptSavedState`] must be passed to a matching [`Self::release_irq_restore`] on the
    /// same CPU before the thread blocks or migrates.
    #[inline]
    pub unsafe fn acquire_irq_save(&self) -> InterruptSavedState {
        // SAFETY: `self.storage.get()` points to an initialized `SpinLock` with write provenance,
        // and the caller upholds the non-reentrant locking preconditions.
        unsafe { cpp_spinlock_acquire_irqsave(self.storage.get().cast::<c_void>()) }
    }

    /// Just like [`Self::release`], but also restores interrupt state.
    ///
    /// # Safety
    ///
    /// This spinlock must currently be held by the calling CPU without outstanding references
    /// protected by this acquisition, and `state` must be the [`InterruptSavedState`] returned by
    /// the corresponding interrupt-saving acquisition on this CPU.
    ///
    /// The allocation backing `self` must remain live until this call returns. The lock word is
    /// released before interrupts are restored, and once released another CPU may acquire the
    /// lock immediately, so this must not be used to release a lock that guards the destruction
    /// of its own containing allocation.
    #[inline]
    pub unsafe fn release_irq_restore(&self, state: InterruptSavedState) {
        // SAFETY: `self.storage.get()` points to an initialized `SpinLock` with write provenance,
        // and the caller guarantees that the calling CPU holds the lock with saved interrupt
        // state `state`.
        unsafe {
            cpp_spinlock_release_irqrestore(self.storage.get().cast::<c_void>(), state);
        }
    }

    /// Asserts (in debug builds) that the spinlock is currently held by the calling CPU.
    ///
    /// # Panics
    ///
    /// In builds with kernel debug assertions enabled, panics if interrupts are not disabled on
    /// the calling CPU or if this spinlock is not held by the calling CPU.
    #[inline]
    pub fn assert_held(&self) {
        // SAFETY: `self.storage.get()` points to an initialized `SpinLock`. C++
        // `SpinLock::AssertHeld()` evaluates `DEBUG_ASSERT(IsHeld())`, which is memory-safe in all
        // states (see `is_held`), and compiles to nothing when kernel debug assertions are
        // disabled.
        unsafe {
            cpp_spinlock_assert_held(self.storage.get().cast_const().cast::<c_void>());
        }
    }
}

impl AsRef<RawSpinlockStorage> for RawSpinlockStorage {
    #[inline]
    fn as_ref(&self) -> &RawSpinlockStorage {
        self
    }
}

impl Default for RawSpinlockStorage {
    fn default() -> Self {
        Self::INIT
    }
}

// SAFETY: `RawSpinlockStorage` is safe to share and access across threads.
unsafe impl Sync for RawSpinlockStorage {}
// SAFETY: `RawSpinlockStorage` is safe to send across threads.
unsafe impl Send for RawSpinlockStorage {}

zr::unsafe_pinned_drop_ffi!(RawSpinlockStorage, cpp_spinlock_destroy);

/// Opaque layout block for the Zircon C++ `MonitoredSpinLock` object.
///
/// Like [`RawSpinlockStorage`], this type intentionally over-aligns to 8 bytes even when
/// `spin_lock_tracing` is disabled.
#[pin_data(PinnedDrop)]
#[repr(C, align(8))]
pub struct RawMonitoredSpinlockStorage {
    storage: zr::OpaqueBytes<RAW_SPINLOCK_SIZE>,
}

impl RawMonitoredSpinlockStorage {
    #[allow(clippy::declare_interior_mutable_const)]
    pub const INIT: Self = Self::const_init();

    /// Statically initializes a `RawMonitoredSpinlockStorage` in constant context.
    pub const fn const_init() -> Self {
        Self { storage: zr::OpaqueBytes::new([0u8; RAW_SPINLOCK_SIZE]) }
    }
}

impl Default for RawMonitoredSpinlockStorage {
    fn default() -> Self {
        Self::INIT
    }
}

// SAFETY: `RawMonitoredSpinlockStorage` is safe to share and access across threads.
unsafe impl Sync for RawMonitoredSpinlockStorage {}
// SAFETY: `RawMonitoredSpinlockStorage` is safe to send across threads.
unsafe impl Send for RawMonitoredSpinlockStorage {}

zr::unsafe_pinned_drop_ffi!(RawMonitoredSpinlockStorage, cpp_monitored_spinlock_destroy);

pub struct IrqSavePolicy;

// SAFETY: `cpp_spinlock_acquire_irqsave` (or `cpp_ksync_interrupt_save` +
// `cpp_spinlock_acquire_no_irqsave`) and `cpp_spinlock_release_irqrestore` enforce mutual
// exclusion on `RawSpinlockStorage`.
unsafe impl LockPolicy<RawSpinlockStorage> for IrqSavePolicy {
    type AcquireArgs = ();
    type GuardState = InterruptSavedState;

    #[inline]
    unsafe fn pre_validate(_lock: &RawSpinlockStorage, state: &mut Self::GuardState, _args: ()) {
        // SAFETY: Saving and disabling interrupts is safe when paired with a matching release.
        *state = unsafe { cpp_ksync_interrupt_save() };
    }

    #[inline]
    unsafe fn pre_reacquire(_lock: &RawSpinlockStorage, state: &mut Self::GuardState) {
        // SAFETY: Saving and disabling interrupts is safe when paired with a matching release.
        *state = unsafe { cpp_ksync_interrupt_save() };
    }

    #[inline]
    unsafe fn acquire(lock: &RawSpinlockStorage, state: &mut Self::GuardState, _args: ()) {
        // SAFETY: The FFI call is safe because the lock is initialized.
        *state = unsafe { cpp_spinlock_acquire_irqsave(lock.as_mut_ptr()) };
    }

    #[inline]
    unsafe fn post_validate_acquire(
        lock: &RawSpinlockStorage,
        _state: &mut Self::GuardState,
        _args: (),
    ) {
        // SAFETY: The lock is initialized and interrupts were disabled in `pre_validate`.
        unsafe {
            cpp_spinlock_acquire_no_irqsave(lock.as_mut_ptr());
        }
    }

    #[inline]
    unsafe fn reacquire(lock: &RawSpinlockStorage, state: &mut Self::GuardState) {
        // SAFETY: Caller upholds the `LockPolicy::reacquire` contract.
        unsafe {
            Self::acquire(lock, state, ());
        }
    }

    #[inline]
    unsafe fn post_validate_reacquire(lock: &RawSpinlockStorage, _state: &mut Self::GuardState) {
        // SAFETY: The lock is initialized and interrupts were disabled in `pre_reacquire`.
        unsafe {
            cpp_spinlock_acquire_no_irqsave(lock.as_mut_ptr());
        }
    }

    #[inline]
    unsafe fn release(lock: &RawSpinlockStorage, state: &mut Self::GuardState) {
        // SAFETY: The FFI call is safe because the lock is initialized and held by the current
        // CPU with saved interrupt state `*state`.
        unsafe {
            cpp_spinlock_release_irqrestore(lock.as_mut_ptr(), *state);
        }
    }
}

// SAFETY: `IrqSavePolicy` enforces exclusive mutual exclusion on `RawSpinlockStorage`.
unsafe impl ExclusiveLockPolicy<RawSpinlockStorage> for IrqSavePolicy {}

pub struct NoIrqSavePolicy;

// SAFETY: `cpp_spinlock_acquire_no_irqsave` and `cpp_spinlock_release_no_irqrestore` enforce
// mutual exclusion on `RawSpinlockStorage`.
unsafe impl LockPolicy<RawSpinlockStorage> for NoIrqSavePolicy {
    type AcquireArgs = ();
    type GuardState = ();

    #[inline]
    unsafe fn acquire(lock: &RawSpinlockStorage, _state: &mut Self::GuardState, _args: ()) {
        // SAFETY: The FFI call is safe because the lock is initialized.
        unsafe { cpp_spinlock_acquire_no_irqsave(lock.as_mut_ptr()) }
    }

    #[inline]
    unsafe fn reacquire(lock: &RawSpinlockStorage, state: &mut Self::GuardState) {
        // SAFETY: Caller upholds the `LockPolicy::reacquire` contract.
        unsafe {
            Self::acquire(lock, state, ());
        }
    }

    #[inline]
    unsafe fn release(lock: &RawSpinlockStorage, _state: &mut Self::GuardState) {
        // SAFETY: The FFI call is safe because the lock is initialized and held by the current
        // CPU.
        unsafe {
            cpp_spinlock_release_no_irqrestore(lock.as_mut_ptr());
        }
    }
}

// SAFETY: `NoIrqSavePolicy` enforces exclusive mutual exclusion on `RawSpinlockStorage`.
unsafe impl ExclusiveLockPolicy<RawSpinlockStorage> for NoIrqSavePolicy {}

/// Saved interrupt state and source tag for a held `RawMonitoredSpinlock`.
#[derive(Copy, Clone, Default)]
pub struct MonitoredSpinlockGuardState {
    pub interrupt_state: InterruptSavedState,
    pub tag: ksync::SourceTag,
}

// SAFETY: `cpp_monitored_spinlock_acquire_irqsave` (or `cpp_ksync_interrupt_save` +
// `cpp_monitored_spinlock_acquire_no_irqsave`) and `cpp_monitored_spinlock_release_irqrestore`
// enforce mutual exclusion on `RawMonitoredSpinlockStorage`.
unsafe impl LockPolicy<RawMonitoredSpinlockStorage> for IrqSavePolicy {
    type AcquireArgs = ksync::SourceTag;
    type GuardState = MonitoredSpinlockGuardState;

    #[inline]
    unsafe fn pre_validate(
        _lock: &RawMonitoredSpinlockStorage,
        state: &mut Self::GuardState,
        tag: Self::AcquireArgs,
    ) {
        // SAFETY: Saving and disabling interrupts is safe when paired with a matching release.
        let interrupt_state = unsafe { cpp_ksync_interrupt_save() };
        *state = MonitoredSpinlockGuardState { interrupt_state, tag };
    }

    #[inline]
    unsafe fn pre_reacquire(_lock: &RawMonitoredSpinlockStorage, state: &mut Self::GuardState) {
        // SAFETY: Saving and disabling interrupts is safe when paired with a matching release.
        state.interrupt_state = unsafe { cpp_ksync_interrupt_save() };
    }

    #[inline]
    unsafe fn acquire(
        lock: &RawMonitoredSpinlockStorage,
        state: &mut Self::GuardState,
        tag: Self::AcquireArgs,
    ) {
        // SAFETY: The FFI call is safe because the lock is initialized and `tag` points to a
        // static null-terminated C string.
        let interrupt_state =
            unsafe { cpp_monitored_spinlock_acquire_irqsave(lock.as_mut_ptr(), tag.as_ptr()) };
        *state = MonitoredSpinlockGuardState { interrupt_state, tag };
    }

    #[inline]
    unsafe fn post_validate_acquire(
        lock: &RawMonitoredSpinlockStorage,
        _state: &mut Self::GuardState,
        tag: Self::AcquireArgs,
    ) {
        // SAFETY: The lock is initialized, interrupts were disabled in `pre_validate`, and `tag`
        // points to a static null-terminated C string.
        unsafe {
            cpp_monitored_spinlock_acquire_no_irqsave(lock.as_mut_ptr(), tag.as_ptr());
        }
    }

    #[inline]
    unsafe fn reacquire(lock: &RawMonitoredSpinlockStorage, state: &mut Self::GuardState) {
        // SAFETY: Same safety requirements as `acquire`.
        state.interrupt_state = unsafe {
            cpp_monitored_spinlock_acquire_irqsave(lock.as_mut_ptr(), state.tag.as_ptr())
        };
    }

    #[inline]
    unsafe fn post_validate_reacquire(
        lock: &RawMonitoredSpinlockStorage,
        state: &mut Self::GuardState,
    ) {
        // SAFETY: The lock is initialized, interrupts were disabled in `pre_reacquire`, and
        // `state.tag` points to a static null-terminated C string.
        unsafe {
            cpp_monitored_spinlock_acquire_no_irqsave(lock.as_mut_ptr(), state.tag.as_ptr());
        }
    }

    #[inline]
    unsafe fn release(lock: &RawMonitoredSpinlockStorage, state: &mut Self::GuardState) {
        // SAFETY: The FFI call is safe because the lock is initialized and held by the current
        // CPU with saved interrupt state `state.interrupt_state`.
        unsafe {
            cpp_monitored_spinlock_release_irqrestore(lock.as_mut_ptr(), state.interrupt_state);
        }
    }
}

// SAFETY: `IrqSavePolicy` enforces exclusive mutual exclusion on `RawMonitoredSpinlockStorage`.
unsafe impl ExclusiveLockPolicy<RawMonitoredSpinlockStorage> for IrqSavePolicy {}

// SAFETY: `cpp_monitored_spinlock_acquire_no_irqsave` and
// `cpp_monitored_spinlock_release_no_irqrestore` enforce mutual exclusion on
// `RawMonitoredSpinlockStorage`.
unsafe impl LockPolicy<RawMonitoredSpinlockStorage> for NoIrqSavePolicy {
    type AcquireArgs = ksync::SourceTag;
    type GuardState = ksync::SourceTag;

    #[inline]
    unsafe fn acquire(
        lock: &RawMonitoredSpinlockStorage,
        state: &mut Self::GuardState,
        tag: Self::AcquireArgs,
    ) {
        // SAFETY: The FFI call is safe because the lock is initialized and `tag` points to a
        // static null-terminated C string.
        unsafe {
            cpp_monitored_spinlock_acquire_no_irqsave(lock.as_mut_ptr(), tag.as_ptr());
        }
        *state = tag;
    }

    #[inline]
    unsafe fn reacquire(lock: &RawMonitoredSpinlockStorage, state: &mut Self::GuardState) {
        // SAFETY: Same safety requirements as `acquire`.
        unsafe {
            cpp_monitored_spinlock_acquire_no_irqsave(lock.as_mut_ptr(), state.as_ptr());
        }
    }

    #[inline]
    unsafe fn release(lock: &RawMonitoredSpinlockStorage, _state: &mut Self::GuardState) {
        // SAFETY: The FFI call is safe because the lock is initialized and held by the current
        // CPU.
        unsafe {
            cpp_monitored_spinlock_release_no_irqrestore(lock.as_mut_ptr());
        }
    }
}

// SAFETY: `NoIrqSavePolicy` enforces exclusive mutual exclusion on `RawMonitoredSpinlockStorage`.
unsafe impl ExclusiveLockPolicy<RawMonitoredSpinlockStorage> for NoIrqSavePolicy {}

// SAFETY: `RawSpinlockStorage` is a physical kernel spinlock with a unique storage address.
unsafe impl RawLock for RawSpinlockStorage {
    type DefaultPolicy = IrqSavePolicy;

    /// # Safety
    ///
    /// `class_id` must be null or a valid static `LockClassId` that outlives the initialized lock.
    #[inline]
    unsafe fn init(class_id: *const c_void) -> impl PinInit<Self, core::convert::Infallible> {
        zr::pin_init_ffi!(cpp_spinlock_init, class_id)
    }

    #[inline]
    fn as_mut_ptr(&self) -> *mut c_void {
        self.storage.get().cast::<c_void>()
    }
}

impl ConstRawLock for RawSpinlockStorage {
    const INIT: Self = Self::INIT;
}

impl RawAssertHeldLock for RawSpinlockStorage {
    #[inline]
    fn assert_held(&self) {
        Self::assert_held(self);
    }
}

impl RawUntrackedLock for RawSpinlockStorage {
    #[inline]
    unsafe fn release(&self) {
        // SAFETY: Caller upholds the `RawSpinlockStorage::release` contract.
        unsafe {
            Self::release(self);
        }
    }
}

impl RawUntrackedIrqLock for RawSpinlockStorage {
    type InterruptState = InterruptSavedState;

    #[inline]
    unsafe fn release_irq_restore(&self, state: Self::InterruptState) {
        // SAFETY: Caller upholds the `RawSpinlockStorage::release_irq_restore` contract.
        unsafe {
            Self::release_irq_restore(self, state);
        }
    }
}

// SAFETY: `RawMonitoredSpinlockStorage` is a physical kernel spinlock with a unique storage
// address.
unsafe impl RawLock for RawMonitoredSpinlockStorage {
    type DefaultPolicy = IrqSavePolicy;

    /// # Safety
    ///
    /// `class_id` must be null or a valid static `LockClassId` that outlives the initialized lock.
    #[inline]
    unsafe fn init(class_id: *const c_void) -> impl PinInit<Self, core::convert::Infallible> {
        zr::pin_init_ffi!(cpp_monitored_spinlock_init, class_id)
    }

    #[inline]
    fn as_mut_ptr(&self) -> *mut c_void {
        self.storage.get().cast::<c_void>()
    }
}

impl ConstRawLock for RawMonitoredSpinlockStorage {
    const INIT: Self = Self::INIT;
}

impl_lockdep_tracked_lock_policy!(IrqSavePolicy);
impl_lockdep_tracked_lock_policy!(NoIrqSavePolicy);

/// Lockdep-tracked kernel spinlock matching the Zircon C++ `Lock<SpinLock>` layout.
pub type RawSpinlock =
    LockdepTracked<RawSpinlockStorage, IrqSavePolicy, { lockdep::LOCK_FLAGS_IRQ_SAFE }>;

/// Lockdep-tracked kernel monitored spinlock matching the Zircon C++ `Lock<MonitoredSpinLock>`
/// layout.
pub type RawMonitoredSpinlock =
    LockdepTracked<RawMonitoredSpinlockStorage, IrqSavePolicy, { lockdep::LOCK_FLAGS_IRQ_SAFE }>;

const _: () = {
    #[cfg(with_lock_dep)]
    const BASE_SIZE: usize = 8;
    #[cfg(not(with_lock_dep))]
    const BASE_SIZE: usize = 0;

    // `RawSpinlockStorage` is `#[repr(C, align(8))]`, so the 4-byte non-tracing storage is padded
    // to 8 bytes.
    const EXPECTED_STORAGE_SIZE: usize = if RAW_SPINLOCK_SIZE == 4 { 8 } else { 16 };
    const EXPECTED_SPINLOCK_SIZE: usize = BASE_SIZE + EXPECTED_STORAGE_SIZE;

    assert!(core::mem::size_of::<RawSpinlockStorage>() == EXPECTED_STORAGE_SIZE);
    assert!(core::mem::align_of::<RawSpinlockStorage>() == 8);
    assert!(core::mem::size_of::<RawMonitoredSpinlockStorage>() == EXPECTED_STORAGE_SIZE);
    assert!(core::mem::align_of::<RawMonitoredSpinlockStorage>() == 8);
    assert!(core::mem::size_of::<RawSpinlock>() == EXPECTED_SPINLOCK_SIZE);
    assert!(core::mem::align_of::<RawSpinlock>() == 8);
    assert!(core::mem::size_of::<RawMonitoredSpinlock>() == EXPECTED_SPINLOCK_SIZE);
    assert!(core::mem::align_of::<RawMonitoredSpinlock>() == 8);
};
