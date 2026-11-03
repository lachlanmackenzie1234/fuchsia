// Copyright 2020 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

//! Kernel spinlock unit tests.

/// SpinLock tests.
#[cfg(ktest)]
#[unittest::suite(name = "spinlock")]
mod tests {
    use crate::arch_rs::InterruptDisableGuard;
    use core::pin::Pin;
    use ksync::{InterruptSavedState, KMutex, RawSpinlockStorage, TryAcquireResult};
    use pin_init::{PinInit, pin_init, stack_pin_init};
    use unittest::{expect_eq, expect_false, expect_true};

    /// Verifies acquiring and releasing a spinlock while saving and restoring interrupt state.
    #[test]
    fn spinlock_lock_unlock() {
        stack_pin_init!(let spinlock = RawSpinlockStorage::new());

        // SAFETY: `spinlock` is not held on the current CPU and is released immediately below.
        let mut state = unsafe { spinlock.acquire_irq_save() };
        // SAFETY: `spinlock` is held on the current CPU with saved interrupt state `state`.
        unsafe {
            spinlock.release_irq_restore(state);
        }

        // SAFETY: `spinlock` is not held on the current CPU and is released immediately below.
        state = unsafe { spinlock.acquire_irq_save() };
        // SAFETY: `spinlock` is held on the current CPU with saved interrupt state `state`.
        unsafe {
            spinlock.release_irq_restore(state);
        }
    }

    /// Verifies non-blocking spinlock acquisition with `try_acquire`.
    #[test]
    fn spinlock_trylock() {
        stack_pin_init!(let spinlock = RawSpinlockStorage::new());
        let _interrupt_guard = InterruptDisableGuard::new();

        // SAFETY: Interrupts are disabled by `_interrupt_guard`, and the acquired lock is
        // released below before `_interrupt_guard` is dropped.
        expect_eq!(TryAcquireResult::Acquired, unsafe { spinlock.try_acquire() }, "TryAcquire");
        // SAFETY: Interrupts are disabled by `_interrupt_guard`; `try_acquire` observes the lock
        // is already held and returns `Failed` without re-acquiring.
        expect_eq!(
            TryAcquireResult::Failed,
            unsafe { spinlock.try_acquire() },
            "TryAcquire second time"
        );
        // SAFETY: Interrupts are disabled and `spinlock` is held on the current CPU.
        unsafe {
            spinlock.release();
        }

        // SAFETY: Interrupts are disabled by `_interrupt_guard`, and the acquired lock is
        // released below before `_interrupt_guard` is dropped.
        expect_eq!(TryAcquireResult::Acquired, unsafe { spinlock.try_acquire() }, "TryAcquire");
        // SAFETY: Interrupts are disabled and `spinlock` is held on the current CPU.
        unsafe {
            spinlock.release();
        }
    }

    /// Verifies `is_held` before, during, and after holding a spinlock.
    #[test]
    fn spinlock_is_held() {
        stack_pin_init!(let spinlock = RawSpinlockStorage::new());
        let _interrupt_guard = InterruptDisableGuard::new();

        expect_false!(spinlock.is_held(), "Lock not held");
        // SAFETY: Interrupts are disabled by `_interrupt_guard` and `spinlock` is not held.
        unsafe {
            spinlock.acquire();
        }
        expect_true!(spinlock.is_held(), "Lock held");
        // SAFETY: Interrupts are disabled and `spinlock` is held on the current CPU.
        unsafe {
            spinlock.release();
        }
        expect_false!(spinlock.is_held(), "Lock not held");
    }

    /// Verifies `assert_held` succeeds when the spinlock is held.
    #[test]
    fn spinlock_assert_held() {
        stack_pin_init!(let spinlock = RawSpinlockStorage::new());

        // SAFETY: `spinlock` is not held on the current CPU and is released below.
        let state = unsafe { spinlock.acquire_irq_save() };
        spinlock.assert_held(); // Lock is held: this should be a no-op.
        // SAFETY: `spinlock` is held on the current CPU with saved interrupt state `state`.
        unsafe {
            spinlock.release_irq_restore(state);
        }
    }

    /// A struct with a guarded value.
    #[ksync::guarded]
    struct ObjectWithLock {
        #[mutex]
        lock: KMutex<RawSpinlockStorage>,
        #[guarded_by(lock)]
        val: i32,
        state: InterruptSavedState,
    }

    impl ObjectWithLock {
        fn new() -> impl PinInit<Self, core::convert::Infallible> {
            pin_init!(Self {
                lock <- KMutex::init(),
                val: 0.into(),
                state: InterruptSavedState::default(),
            })
        }

        /// # Safety
        ///
        /// The calling CPU must not already hold `self.lock`, and the acquisition must be paired
        /// with a matching `release_irq_restore` using `self.state` on the same CPU.
        unsafe fn take_lock(self: Pin<&mut Self>) {
            let this = self.project();
            // SAFETY: The caller guarantees `this.lock` is not held by the calling CPU and will
            // be released with `*this.state` on the same CPU.
            *this.state = unsafe { this.lock.raw_mutex().acquire_irq_save() };
        }
    }

    /// Verifies that `assert_held` grants compile-time access to `ksync`-guarded state.
    #[test]
    fn spinlock_assert_held_compile_test() {
        stack_pin_init!(let object = ObjectWithLock::new());

        // This shouldn't compile with ksync's compile-time lock checking.
        #[cfg(enable_errors)]
        {
            object.val = 3;
        }

        // We take the lock, but Rust's type system can't see it.
        // SAFETY: `object.lock` is not held and is released via `release_irq_restore` below.
        unsafe {
            object.as_mut().take_lock();
        }

        // Without the assertion, Rust will object to setting "val".
        #[cfg(not(enable_errors))]
        // SAFETY: `take_lock()` acquired `object.lock` with interrupts disabled on the current CPU
        // above, `token` does not outlive the lock hold, and no other borrows of guarded fields on
        // `object` exist.
        let mut token = unsafe { object.lock.assert_held() };
        // SAFETY: `token` was obtained from `object.lock` on this `object` instance.
        *unsafe { object.guard_lock_mut(&mut token) }.val_mut() = 3;

        // Without the assertion, Rust will object to releasing the lock.
        // SAFETY: `object.lock` is held on the current CPU with saved state `object.state` and no
        // borrows of data guarded by this acquisition remain live.
        unsafe {
            object.lock.release_irq_restore(object.state, token);
        }
    }
}
