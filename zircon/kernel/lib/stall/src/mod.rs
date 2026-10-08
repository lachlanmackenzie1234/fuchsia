// Copyright 2024 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

//! Memory pressure stall accounting.
//!
//! Every CPU owns a [`StallAccumulator`], stored inside its `struct percpu`, which tracks how much
//! time that CPU spent running stalling and progressing threads. `StallAggregator` periodically
//! flushes them and turns them into the system-wide stats reported by `ZX_INFO_MEMORY_STALL`.

use crate::arch_rs::InterruptDisableGuard;
use crate::platform_rs::timer::{DurationMono, InstantMono, current_mono_time};
use core::convert::Infallible;
use core::mem::MaybeUninit;
use pin_init::{InPlaceWrite as _, PinInit, pin_data, pin_init};

/// Stall measurements accumulated by a [`StallAccumulator`] since the last flush.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AccumulatorStats {
    /// Monotonic time spent with `num_contributors_stalling > 0`.
    pub total_time_stall_some: DurationMono,

    /// Monotonic time spent with `num_contributors_stalling > 0 &&
    /// num_contributors_progressing == 0`.
    pub total_time_stall_full: DurationMono,

    /// Monotonic time spent with `num_contributors_progressing > 0 ||
    /// num_contributors_stalling > 0`.
    pub total_time_active: DurationMono,
}

/// Maintains per-CPU stall timers in real time.
///
/// Its counters are conceptually continuous. In fact, we update the saved state whenever the
/// conditions change, so we can always compute the current value by extrapolating.
///
/// With respect to stall contributions, threads are always in one of these three states:
///  - Not contributing at all to any accumulator.
///  - Contributing to an accumulator as a progressing thread.
///  - Contributing to an accumulator as a stalling thread.
#[ksync::guarded]
#[pin_data]
#[repr(C)]
pub struct StallAccumulator {
    #[mutex]
    lock: ksync::KMutex<ksync::RawSpinlock>,

    /// Number of progressing threads currently tracked by this structure.
    #[guarded_by(lock)]
    num_contributors_progressing: usize,

    /// Number of stalling threads currently tracked by this structure.
    #[guarded_by(lock)]
    num_contributors_stalling: usize,

    /// Timestamp of the last `consolidate` call.
    #[guarded_by(lock)]
    last_consolidate_time: InstantMono,

    /// Accumulated totals at the time of the last update.
    #[guarded_by(lock)]
    accumulated_stats: AccumulatorStats,
}

fn consolidate(
    num_contributors_progressing: usize,
    num_contributors_stalling: usize,
    last_consolidate_time: &mut InstantMono,
    accumulated_stats: &mut AccumulatorStats,
) {
    let now = current_mono_time();
    let time_delta = now - *last_consolidate_time;

    if num_contributors_stalling > 0 {
        accumulated_stats.total_time_stall_some =
            accumulated_stats.total_time_stall_some + time_delta;
    }

    if num_contributors_stalling > 0 && num_contributors_progressing == 0 {
        accumulated_stats.total_time_stall_full =
            accumulated_stats.total_time_stall_full + time_delta;
    }

    if num_contributors_progressing > 0 || num_contributors_stalling > 0 {
        accumulated_stats.total_time_active = accumulated_stats.total_time_active + time_delta;
    }

    *last_consolidate_time = now;
}

impl StallAccumulator {
    pub fn init() -> impl PinInit<Self, Infallible> {
        pin_init!(Self {
            lock <- ksync::KMutex::init(),
            num_contributors_progressing: 0usize.into(),
            num_contributors_stalling: 0usize.into(),
            last_consolidate_time: InstantMono::ZERO.into(),
            accumulated_stats: AccumulatorStats::default().into(),
        })
    }

    fn update_with_irq_disabled(
        &self,
        op_contributors_progressing: i32,
        op_contributors_stalling: i32,
    ) {
        // Check argument range.
        debug_assert!((-1..=1).contains(&op_contributors_progressing));
        debug_assert!((-1..=1).contains(&op_contributors_stalling));

        ksync::lock!(let mut guard = self.lock_lock_policy::<ksync::NoIrqSavePolicy>());
        let fields = guard.as_mut().fields_mut();
        consolidate(
            *fields.num_contributors_progressing,
            *fields.num_contributors_stalling,
            fields.last_consolidate_time,
            fields.accumulated_stats,
        );

        // Apply variations.
        *fields.num_contributors_progressing = fields
            .num_contributors_progressing
            .wrapping_add_signed(op_contributors_progressing as isize);
        *fields.num_contributors_stalling =
            fields.num_contributors_stalling.wrapping_add_signed(op_contributors_stalling as isize);

        // Check that we are counting correctly and we never decrement below zero.
        debug_assert_ne!(*fields.num_contributors_progressing, usize::MAX);
        debug_assert_ne!(*fields.num_contributors_stalling, usize::MAX);
    }

    /// Alter contributor counts by the given amount.
    ///
    /// Only values between -1 and +1 are accepted.
    pub fn update(&self, op_contributors_progressing: i32, op_contributors_stalling: i32) {
        let _guard = InterruptDisableGuard::new();
        self.update_with_irq_disabled(op_contributors_progressing, op_contributors_stalling);
    }

    /// Reads the current stats and resets them to zero.
    pub fn flush(&self) -> AccumulatorStats {
        ksync::lock!(let mut guard = self.lock_lock());
        let fields = guard.as_mut().fields_mut();
        consolidate(
            *fields.num_contributors_progressing,
            *fields.num_contributors_stalling,
            fields.last_consolidate_time,
            fields.accumulated_stats,
        );

        core::mem::take(fields.accumulated_stats)
    }
}

/// Initializes the `StallAccumulator` held by a `struct percpu`.
///
/// # Safety
///
/// `accumulator` must point to writable storage that is correctly sized and aligned for a
/// `StallAccumulator`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_stall_accumulator_init(
    accumulator: *mut MaybeUninit<StallAccumulator>,
) {
    // SAFETY: accumulator is a correctly sized and aligned ptr.
    unsafe {
        let Ok(_) = accumulator.as_mut_unchecked().write_pin_init(StallAccumulator::init());
    }
}

/// Alters the accumulator's contributor counts by the given amount.
#[unsafe(no_mangle)]
pub extern "C" fn rust_stall_accumulator_update(
    accumulator: &StallAccumulator,
    op_contributors_progressing: i32,
    op_contributors_stalling: i32,
) {
    accumulator.update(op_contributors_progressing, op_contributors_stalling);
}

/// Alters the accumulator's contributor counts by the given amount, assuming interrupts are
/// already disabled.
#[unsafe(no_mangle)]
pub extern "C" fn rust_stall_accumulator_update_no_irq(
    accumulator: &StallAccumulator,
    op_contributors_progressing: i32,
    op_contributors_stalling: i32,
) {
    accumulator.update_with_irq_disabled(op_contributors_progressing, op_contributors_stalling);
}

/// Reads the accumulator's current stats into `out_stats` and resets them to zero.
#[unsafe(no_mangle)]
pub extern "C" fn rust_stall_accumulator_flush(
    accumulator: &StallAccumulator,
    out_stats: &mut AccumulatorStats,
) {
    *out_stats = accumulator.flush();
}

/// Tests for the per-CPU memory stall accumulator.
#[cfg(ktest)]
#[unittest::suite(name = "stall_rust")]
mod stall_tests {
    use super::StallAccumulator;
    use crate::platform_rs::timer::DurationMono;
    use crate::top::debug::spin_usecs;
    use pin_init::stack_pin_init;
    use unittest::{assert_ge, expect_eq, expect_ge, expect_le};

    /// Verifies that non-stalling threads only generate `active` time while running.
    #[test]
    fn non_stalling_time() {
        stack_pin_init!(let accumulator = StallAccumulator::init());

        // Progressing non-stalling time should make only the `active` timer grow.
        accumulator.update(1, 0);
        spin_usecs(1_000);
        accumulator.update(-1, 0);
        let stats1 = accumulator.flush();
        expect_le!(
            DurationMono::from_millis(1).into_nanos(),
            stats1.total_time_active.into_nanos()
        );
        expect_eq!(0, stats1.total_time_stall_some.into_nanos());
        expect_eq!(0, stats1.total_time_stall_full.into_nanos());

        // Non-progressing non-stalling time should not make any timer grow.
        spin_usecs(1_000);
        let stats2 = accumulator.flush();
        expect_eq!(0, stats2.total_time_active.into_nanos());
        expect_eq!(0, stats2.total_time_stall_some.into_nanos());
        expect_eq!(0, stats2.total_time_stall_full.into_nanos());
    }

    /// Verifies that the `some` and `full` stall timers grow with stalling threads.
    #[test]
    fn stalling_time() {
        stack_pin_init!(let accumulator = StallAccumulator::init());

        // Verifies that a single stalling thread makes all the timers grow.
        // Checks: active == some == full >= 1 ms.
        accumulator.update(0, 1);
        spin_usecs(1_000);
        let stats1 = accumulator.flush();
        expect_eq!(
            stats1.total_time_active.into_nanos(),
            stats1.total_time_stall_some.into_nanos()
        );
        expect_eq!(
            stats1.total_time_stall_some.into_nanos(),
            stats1.total_time_stall_full.into_nanos()
        );
        expect_ge!(
            stats1.total_time_stall_full.into_nanos(),
            DurationMono::from_millis(1).into_nanos()
        );

        // Verifies that adding a second non-stalling thread makes `full` stop growing, but not the
        // others. Checks: active == some >= 1 ms && (some - 1 ms) >= full
        accumulator.update(1, 0);
        spin_usecs(1_000);
        let stats2 = accumulator.flush();
        expect_eq!(
            stats2.total_time_active.into_nanos(),
            stats2.total_time_stall_some.into_nanos()
        );
        assert_ge!(
            stats2.total_time_stall_some.into_nanos(),
            DurationMono::from_millis(1).into_nanos()
        );
        expect_ge!(
            (stats2.total_time_stall_some - DurationMono::from_millis(1)).into_nanos(),
            stats2.total_time_stall_full.into_nanos()
        );

        // Cleanup.
        accumulator.update(-1, -1);
    }
}
