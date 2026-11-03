// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#ifndef ZIRCON_KERNEL_LIB_KSYNC_BINDINGS_H_
#define ZIRCON_KERNEL_LIB_KSYNC_BINDINGS_H_

#include <stdint.h>
#include <zircon/time.h>

#include <arch/interrupt.h>
#include <kernel/brwlock.h>
#include <kernel/mutex.h>
#include <kernel/spinlock.h>
#include <lockdep/lockdep.h>

extern "C" {

interrupt_saved_state_t cpp_ksync_interrupt_save();
void cpp_mutex_destroy(Mutex* lock);
void cpp_mutex_acquire(Mutex* lock);
void cpp_mutex_acquire_with_spin_max_duration(Mutex* lock, zx_duration_mono_t spin_max_duration);
void cpp_mutex_release(Mutex* lock);
bool cpp_mutex_is_contested(Mutex* lock);
bool cpp_mutex_is_held(const Mutex* lock);
bool cpp_critical_mutex_acquire(CriticalMutex* lock);
void cpp_critical_mutex_release(CriticalMutex* lock, bool should_clear);
void cpp_spinlock_init(SpinLock* lock, const void* class_id);
void cpp_spinlock_destroy(SpinLock* lock);
interrupt_saved_state_t cpp_spinlock_acquire_irqsave(SpinLock* lock);
void cpp_spinlock_release_irqrestore(SpinLock* lock, interrupt_saved_state_t state);
void cpp_spinlock_acquire_no_irqsave(SpinLock* lock);
bool cpp_spinlock_try_acquire(SpinLock* lock);
void cpp_spinlock_release_no_irqrestore(SpinLock* lock);
bool cpp_spinlock_is_held(const SpinLock* lock);
void cpp_spinlock_assert_held(const SpinLock* lock);
void cpp_monitored_spinlock_init(MonitoredSpinLock* lock, const void* class_id);
void cpp_monitored_spinlock_destroy(MonitoredSpinLock* lock);
interrupt_saved_state_t cpp_monitored_spinlock_acquire_irqsave(MonitoredSpinLock* lock,
                                                               const char* name);
void cpp_monitored_spinlock_release_irqrestore(MonitoredSpinLock* lock,
                                               interrupt_saved_state_t state);
void cpp_monitored_spinlock_acquire_no_irqsave(MonitoredSpinLock* lock, const char* name);
void cpp_monitored_spinlock_release_no_irqrestore(MonitoredSpinLock* lock);
void cpp_brwlock_pi_init(BrwLockPi* lock, const void* class_id);
void cpp_brwlock_pi_destroy(BrwLockPi* lock);
void cpp_brwlock_pi_acquire_read(BrwLockPi* lock);
void cpp_brwlock_pi_release_read(BrwLockPi* lock);
void cpp_brwlock_pi_acquire_write(BrwLockPi* lock);
void cpp_brwlock_pi_release_write(BrwLockPi* lock);
void cpp_lock_dep_acquire(void* lock, const void* class_id, void* entry_storage,
                          lockdep::LockFlags flags, uintptr_t order, bool assert_nestable);
void cpp_lock_dep_release(void* entry_storage, lockdep::LockFlags flags);
void cpp_lock_validate_release(void* entry_storage, lockdep::LockFlags flags);
void cpp_lock_validate_acquire(void* entry_storage, lockdep::LockFlags flags);

}  // extern "C"

#endif  // ZIRCON_KERNEL_LIB_KSYNC_BINDINGS_H_
