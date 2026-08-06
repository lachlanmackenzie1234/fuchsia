// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT
//
// Ported from zircon/kernel/dev/pdev/hw_watchdog/hw_watchdog.cc

use core::sync::atomic::{AtomicPtr, Ordering};
#[cfg(ktest)]
use unittest as _;
use zx_status::Status;

/// Hardware watchdog operations table defining the platform-specific callbacks.
#[allow(unpredictable_function_pointer_comparisons)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct PdevWatchdogOps {
    /// Callback to pet the hardware watchdog.
    pub pet: extern "C" fn(),
    /// Callback to set the enabled/disabled state of the watchdog. Returns `zx_status_t`.
    pub set_enabled: extern "C" fn(enabled: bool) -> i32,
    /// Callback returning `true` if the watchdog is currently enabled.
    pub is_enabled: extern "C" fn() -> bool,
    /// Callback returning the nominal timeout period in nanoseconds (`zx_duration_boot_t`).
    pub get_timeout_nsec: extern "C" fn() -> i64,
    /// Callback returning the last successful pet time (`zx_instant_boot_t`).
    pub get_last_pet_time: extern "C" fn() -> i64,
    /// Callback to enable (`true`) or disable (`false`) suppression of future pets.
    pub suppress_petting: extern "C" fn(suppressed: bool),
    /// Callback returning `true` if petting is currently suppressed.
    pub is_petting_suppressed: extern "C" fn() -> bool,
}

const _: () = assert!(core::mem::size_of::<PdevWatchdogOps>() == 56);
const _: () = assert!(core::mem::align_of::<PdevWatchdogOps>() == 8);

static WATCHDOG_OPS: AtomicPtr<PdevWatchdogOps> =
    AtomicPtr::new(core::ptr::addr_of!(DEFAULT_OPS).cast_mut());

extern "C" fn default_pet() {}

extern "C" fn default_set_enabled(_: bool) -> i32 {
    Status::NOT_SUPPORTED.into_raw()
}

extern "C" fn default_is_enabled() -> bool {
    false
}

extern "C" fn default_get_timeout_nsec() -> i64 {
    i64::MAX
}

extern "C" fn default_get_last_pet_time() -> i64 {
    0
}

extern "C" fn default_suppress_petting(_: bool) {}

extern "C" fn default_is_petting_suppressed() -> bool {
    true
}

static DEFAULT_OPS: PdevWatchdogOps = PdevWatchdogOps {
    pet: default_pet,
    set_enabled: default_set_enabled,
    is_enabled: default_is_enabled,
    get_timeout_nsec: default_get_timeout_nsec,
    get_last_pet_time: default_get_last_pet_time,
    suppress_petting: default_suppress_petting,
    is_petting_suppressed: default_is_petting_suppressed,
};

fn get_ops() -> *const PdevWatchdogOps {
    let ops = WATCHDOG_OPS.load(Ordering::Acquire);
    if ops.is_null() { core::ptr::addr_of!(DEFAULT_OPS) } else { ops }
}

/// Registers the hardware watchdog operations table with the PDEV dispatch layer.
///
/// # Safety
///
/// `ops` must be either null (which unregisters the current operations table and
/// falls back to `DEFAULT_OPS`) or a valid pointer to a static `PdevWatchdogOps`
/// structure whose function pointers live for the duration of the kernel's execution.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pdev_register_watchdog(ops: *const PdevWatchdogOps) {
    WATCHDOG_OPS.store(ops.cast_mut(), Ordering::Release);
    core::sync::atomic::fence(Ordering::SeqCst);
}

/// Returns `true` if this platform has a hardware watchdog, `false` otherwise.
///
/// # Safety
///
/// The caller must ensure that `get_ops()` returns a valid pointer to a
/// `PdevWatchdogOps` structure.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hw_watchdog_present() -> bool {
    let ops = get_ops();
    ops != core::ptr::addr_of!(DEFAULT_OPS)
}

/// Pets the hardware watchdog if present and petting is not suppressed.
///
/// # Safety
///
/// The caller must ensure that `get_ops()` returns a valid pointer to a
/// `PdevWatchdogOps` structure whose `pet` function pointer is valid to invoke.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hw_watchdog_pet() {
    // SAFETY: `get_ops()` returns either a pointer to the static `DEFAULT_OPS` table
    // or a pointer to a registered, static `PdevWatchdogOps` table whose `pet` pointer is valid.
    unsafe { ((*get_ops()).pet)() }
}

/// Attempts to enable or disable the hardware watchdog. Note that depending on
/// hardware details, it may not be possible to change its enabled/disabled state.
///
/// # Safety
///
/// The caller must ensure that `get_ops()` returns a valid pointer to a
/// `PdevWatchdogOps` structure whose `set_enabled` function pointer is valid to invoke.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hw_watchdog_set_enabled(enabled: bool) -> i32 {
    // SAFETY: `get_ops()` returns either a pointer to the static `DEFAULT_OPS` table
    // or a pointer to a registered, static `PdevWatchdogOps` table whose `set_enabled` pointer is valid.
    unsafe { ((*get_ops()).set_enabled)(enabled) }
}

/// Returns `true` if this platform has a hardware watchdog and that watchdog is
/// currently enabled.
///
/// # Safety
///
/// The caller must ensure that `get_ops()` returns a valid pointer to a
/// `PdevWatchdogOps` structure whose `is_enabled` function pointer is valid to invoke.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hw_watchdog_is_enabled() -> bool {
    // SAFETY: `get_ops()` returns either a pointer to the static `DEFAULT_OPS` table
    // or a pointer to a registered, static `PdevWatchdogOps` table whose `is_enabled` pointer is valid.
    unsafe { ((*get_ops()).is_enabled)() }
}

/// Returns the nominal timeout period of the hardware watchdog in nanoseconds.
///
/// # Safety
///
/// The caller must ensure that `get_ops()` returns a valid pointer to a
/// `PdevWatchdogOps` structure whose `get_timeout_nsec` function pointer is valid to invoke.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hw_watchdog_get_timeout_nsec() -> i64 {
    // SAFETY: `get_ops()` returns either a pointer to the static `DEFAULT_OPS` table
    // or a pointer to a registered, static `PdevWatchdogOps` table whose `get_timeout_nsec` pointer is valid.
    unsafe { ((*get_ops()).get_timeout_nsec)() }
}

/// Returns the last time at which the hardware watchdog was successfully pet.
///
/// # Safety
///
/// The caller must ensure that `get_ops()` returns a valid pointer to a
/// `PdevWatchdogOps` structure whose `get_last_pet_time` function pointer is valid to invoke.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hw_watchdog_get_last_pet_time() -> i64 {
    // SAFETY: `get_ops()` returns either a pointer to the static `DEFAULT_OPS` table
    // or a pointer to a registered, static `PdevWatchdogOps` table whose `get_last_pet_time` pointer is valid.
    unsafe { ((*get_ops()).get_last_pet_time)() }
}

/// When `suppressed` is `true`, prevent any thread from actually petting the
/// watchdog. Otherwise, permit threads to pet the watchdog. This feature is
/// used when the system is attempting to create a crashlog and reboot during a
/// software watchdog panic. At the start of this process, HW watchdog petting
/// is suppressed to make sure that even if one or more cores is functioning,
/// that they cannot pet the watchdog while the core attempting to reboot is
/// building the crashlog. This way, if the core attempting to reboot somehow
/// locks up, the HW watchdog will fire as a last resort.
///
/// # Safety
///
/// The caller must ensure that `get_ops()` returns a valid pointer to a
/// `PdevWatchdogOps` structure whose `suppress_petting` function pointer is valid to invoke.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hw_watchdog_suppress_petting(suppressed: bool) {
    // SAFETY: `get_ops()` returns either a pointer to the static `DEFAULT_OPS` table
    // or a pointer to a registered, static `PdevWatchdogOps` table whose `suppress_petting` pointer is valid.
    unsafe { ((*get_ops()).suppress_petting)(suppressed) }
}

/// Returns `true` if watchdog petting suppression is enabled, `false` otherwise.
///
/// # Safety
///
/// The caller must ensure that `get_ops()` returns a valid pointer to a
/// `PdevWatchdogOps` structure whose `is_petting_suppressed` function pointer is valid to invoke.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hw_watchdog_is_petting_suppressed() -> bool {
    // SAFETY: `get_ops()` returns either a pointer to the static `DEFAULT_OPS` table
    // or a pointer to a registered, static `PdevWatchdogOps` table whose `is_petting_suppressed` pointer is valid.
    unsafe { ((*get_ops()).is_petting_suppressed)() }
}

/// PDEV hardware watchdog layer kernel tests.
#[cfg(ktest)]
#[unittest::suite(name = "hw_watchdog")]
mod tests {
    use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use unittest::{assert_eq, assert_false, assert_true};

    /// Test default ops dispatch table fallback behavior and default state.
    #[test]
    fn test_pdev_watchdog_default_ops_fallback() {
        // SAFETY: Calling global C-ABI watchdog functions in unit test.
        unsafe {
            assert_false!(hw_watchdog_present());
            assert_eq!(hw_watchdog_set_enabled(true), Status::NOT_SUPPORTED.into_raw());
            assert_false!(hw_watchdog_is_enabled());
            assert_eq!(hw_watchdog_get_timeout_nsec(), i64::MAX);
            assert_eq!(hw_watchdog_get_last_pet_time(), 0);
            assert_true!(hw_watchdog_is_petting_suppressed());

            // Verify default no-op callbacks don't crash when invoked.
            hw_watchdog_pet();
            hw_watchdog_suppress_petting(true);
            hw_watchdog_suppress_petting(false);
        }
    }

    /// Test registration of custom watchdog ops and execution of hooks.
    #[test]
    fn test_pdev_watchdog_registration() {
        static PET_COUNT: AtomicU32 = AtomicU32::new(0);
        static SUPPRESS_STATE: AtomicBool = AtomicBool::new(false);

        extern "C" fn dummy_pet() {
            PET_COUNT.fetch_add(1, Ordering::SeqCst);
        }
        extern "C" fn dummy_set_enabled(_: bool) -> i32 {
            Status::OK.into_raw()
        }
        extern "C" fn dummy_is_enabled() -> bool {
            true
        }
        extern "C" fn dummy_get_timeout_nsec() -> i64 {
            12345
        }
        extern "C" fn dummy_get_last_pet_time() -> i64 {
            67890
        }
        extern "C" fn dummy_suppress_petting(suppressed: bool) {
            SUPPRESS_STATE.store(suppressed, Ordering::SeqCst);
        }
        extern "C" fn dummy_is_petting_suppressed() -> bool {
            SUPPRESS_STATE.load(Ordering::SeqCst)
        }

        static DUMMY_OPS: PdevWatchdogOps = PdevWatchdogOps {
            pet: dummy_pet,
            set_enabled: dummy_set_enabled,
            is_enabled: dummy_is_enabled,
            get_timeout_nsec: dummy_get_timeout_nsec,
            get_last_pet_time: dummy_get_last_pet_time,
            suppress_petting: dummy_suppress_petting,
            is_petting_suppressed: dummy_is_petting_suppressed,
        };

        // SAFETY: Testing registration and invocation of global C-ABI watchdog functions.
        unsafe {
            PET_COUNT.store(0, Ordering::SeqCst);
            SUPPRESS_STATE.store(false, Ordering::SeqCst);

            pdev_register_watchdog(core::ptr::addr_of!(DUMMY_OPS));

            assert_true!(hw_watchdog_present());
            assert_eq!(hw_watchdog_set_enabled(true), Status::OK.into_raw());
            assert_true!(hw_watchdog_is_enabled());
            assert_eq!(hw_watchdog_get_timeout_nsec(), 12345);
            assert_eq!(hw_watchdog_get_last_pet_time(), 67890);
            assert_false!(hw_watchdog_is_petting_suppressed());

            // Verify pet hook execution.
            hw_watchdog_pet();
            assert_eq!(PET_COUNT.load(Ordering::SeqCst), 1);
            hw_watchdog_pet();
            assert_eq!(PET_COUNT.load(Ordering::SeqCst), 2);

            // Verify suppress_petting hook execution and state propagation.
            hw_watchdog_suppress_petting(true);
            assert_true!(SUPPRESS_STATE.load(Ordering::SeqCst));
            assert_true!(hw_watchdog_is_petting_suppressed());

            hw_watchdog_suppress_petting(false);
            assert_false!(SUPPRESS_STATE.load(Ordering::SeqCst));
            assert_false!(hw_watchdog_is_petting_suppressed());

            // Reset back to null/default ops so other tests or kernel state aren't affected.
            pdev_register_watchdog(core::ptr::null());
            assert_false!(hw_watchdog_present());
        }
    }
}
