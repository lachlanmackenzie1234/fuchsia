// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! Thread-safe filesystem test hooks.
//!
//! # Design Architecture & Invariants
//!
//! - **`Hooks<'a>` & `HooksHandle`**: `Hooks<'a>` is the exclusive mutator handle bound to lifetime
//!   `'a` (the test context). `HooksHandle` is shared via `Arc` by `FxFilesystem` options to invoke
//!   registered test callbacks (`on_*` methods) during transaction commits.
//!
//! - **Storage (`HooksInner` & `HooksTable`)**: Registered callback slots (`HooksTable<'a>`) and
//!   deferred cleanup queue (`GarbageBin<'a>`) are held inside `HooksInner<'a>`. `HooksHandle`
//!   manages a raw pointer to `HooksInner<'a>` under `Mutex<State>`.
//!
//! - **Concurrent Reader Guards**: Readers invoke `acquire_guard`, which increments active reader
//!   `count` under lock and returns `ReaderGuard<'a, T>`. Callbacks are executed outside the lock.
//!
//! - **Deferred Reclamation & Lifetime Invariants**: Updating a hook (`set_*`) moves the old `Hook`
//!   into `GarbageBin<'a>`. Deferred cleanups run only when active reader `count` reaches 0 (as
//!   the last active `ReaderGuard` drops). Dropping `Hooks<'a>` detaches `HooksHandle` and blocks
//!   until all active reader guards drop, ensuring closures bound to lifetime `'a` are never
//!   freed while executing or accessed after lifetime `'a` expires.

use crate::object_store::transaction::Transaction;
use anyhow::Error;
use fuchsia_sync::{Condvar, Mutex};
use std::ptr::NonNull;
use std::sync::Arc;

type PreCommitHookFn<'a> = dyn Fn(&Transaction<'_>) -> Result<(), Error> + Send + Sync + 'a;
type SyncHookFn<'a> = dyn Fn() + Send + Sync + 'a;
type CleanupFn<'a> = Box<dyn FnOnce() + Send + Sync + 'a>;

/// Used to register test hooks for an `FxFilesystem`.
pub struct Hooks<'a> {
    handle: Arc<HooksHandle>,
    _phantom: std::marker::PhantomData<&'a ()>,
}

impl<'a> Hooks<'a> {
    /// Creates a new `Hooks` instance and an associated `HooksHandle`.
    pub fn new() -> (Self, Arc<HooksHandle>) {
        let handle = HooksHandle::new(Box::new(HooksInner::default()));
        (Self { handle: handle.clone(), _phantom: std::marker::PhantomData }, handle)
    }

    /// Sets the hook executed before committing a transaction.  This hook is called before
    /// any locks required for the commit are taken, so it's before any transaction locks
    /// are upgraded to write locks, before any guards that are taken by calling
    /// `prepare_commit` for objects involved in the transaction, and before the commit mutex
    /// is acquired.
    pub fn set_pre_commit(
        &mut self,
        hook: impl Fn(&Transaction<'_>) -> Result<(), Error> + Send + Sync + 'a,
    ) {
        self.set_hook::<PreCommitHookFn<'a>>(|table| &mut table.pre_commit, Box::new(hook));
    }

    /// Sets the hook executed before a transaction starts committing.  This hook is called
    /// after calling `prepare_commit` on all objects involved in the transaction (which
    /// might acquire some locks/guards), but before acquiring the commit mutex.
    pub fn set_before_commit(&mut self, hook: impl Fn() + Send + Sync + 'a) {
        self.set_hook::<SyncHookFn<'a>>(|table| &mut table.before_commit, Box::new(hook));
    }

    /// Sets the hook executed when resources are acquired during unlock.  This is called
    /// after acquiring all the keys that might be required from the crypt service.
    pub fn set_unlock_resources_acquired(&mut self, hook: impl Fn() + Send + Sync + 'a) {
        self.set_hook::<SyncHookFn<'a>>(
            |table| &mut table.unlock_resources_acquired,
            Box::new(hook),
        );
    }

    fn set_hook<T: ?Sized + Send + Sync + 'a>(
        &mut self,
        get_hook: impl for<'b> FnOnce(&'b mut HooksTable<'a>) -> &'b mut Hook<T>,
        hook: Box<T>,
    ) {
        let handle = self.handle.clone();
        let state = handle.state.lock();
        let ptr = state.inner.expect("Hooks inner detached");

        // SAFETY: `state.inner` remains valid under `handle.state.lock()`.
        // `&mut self` and `ReaderGuard` containing no `&HooksInner` reference guarantees
        // no aliasing violation against active readers.
        let inner = unsafe { ptr.cast::<HooksInner<'a>>().as_mut() };
        let hook_slot = get_hook(&mut inner.table);

        // SAFETY: `hook_slot` is associated with `inner.bin`. `set` is called under `state.lock()`.
        unsafe {
            hook_slot.set(&inner.bin, state.count, hook);
        }
    }
}

impl Drop for Hooks<'_> {
    fn drop(&mut self) {
        self.handle.detach();
    }
}

#[derive(Default)]
struct State {
    count: u64,
    inner: Option<NonNull<()>>,
}

/// Handle stored on `FxFilesystem` options to trigger registered test hooks.
pub struct HooksHandle {
    state: Mutex<State>,
    condvar: Condvar,
}

impl Default for HooksHandle {
    fn default() -> Self {
        Self { state: Mutex::new(State::default()), condvar: Condvar::new() }
    }
}

// SAFETY: `HooksHandle` manages raw pointer access protected by `Mutex<State>`.
unsafe impl Send for HooksHandle {}
// SAFETY: `HooksHandle` manages raw pointer access protected by `Mutex<State>`.
unsafe impl Sync for HooksHandle {}

impl HooksHandle {
    fn new(inner: Box<HooksInner<'_>>) -> Arc<Self> {
        let inner = NonNull::new(Box::into_raw(inner) as *mut ());
        Arc::new(Self { state: Mutex::new(State { count: 0, inner }), condvar: Condvar::new() })
    }

    /// Invokes the registered `pre_commit` hook, if any.
    pub fn on_pre_commit(&self, transaction: &Transaction<'_>) -> Result<(), Error> {
        if let Some(guard) = self.acquire_guard(|table| &table.pre_commit) {
            (guard.hook)(transaction)
        } else {
            Ok(())
        }
    }

    /// Invokes the registered `before_commit` hook, if any.
    pub fn on_before_commit(&self) {
        if let Some(guard) = self.acquire_guard(|table| &table.before_commit) {
            (guard.hook)();
        }
    }

    /// Invokes the registered `unlock_resources_acquired` hook, if any.
    pub fn on_unlock_resources_acquired(&self) {
        if let Some(guard) = self.acquire_guard(|table| &table.unlock_resources_acquired) {
            (guard.hook)();
        }
    }

    fn detach(&self) {
        let mut state = self.state.lock();
        if let Some(ptr) = state.inner.take() {
            // Wait for all readers to complete before reconstructing Box.
            while state.count > 0 {
                self.condvar.wait(&mut state);
            }

            // SAFETY: `ptr` was created in `HooksHandle::new` via `Box::into_raw`.
            // Now that `state.count == 0` and `state.inner` is `None`, no readers exist
            // or can be created.
            let _inner = unsafe { Box::from_raw(ptr.cast::<HooksInner<'_>>().as_ptr()) };
        }
    }

    fn acquire_guard<'a, T: ?Sized + Send + Sync>(
        &'a self,
        get_hook: impl for<'b> FnOnce(&'b HooksTable<'a>) -> &'b Hook<T>,
    ) -> Option<ReaderGuard<'a, T>> {
        let mut state = self.state.lock();
        if let Some(ptr) = state.inner {
            // SAFETY: `state.inner` remains valid under lock.
            let inner = unsafe { ptr.cast::<HooksInner<'a>>().as_ref() };

            let hook_slot = get_hook(&inner.table);

            if let Some(hook_ptr) = hook_slot.get() {
                state.count += 1;

                // SAFETY: `hook_ptr` remains valid and allocated while `state.count > 0`.
                let hook = unsafe { hook_ptr.as_ref() };

                return Some(ReaderGuard { handle: self, hook });
            }
        }
        None
    }
}

// Private implementation details

struct ReaderGuard<'a, T: ?Sized> {
    handle: &'a HooksHandle,
    pub hook: &'a T,
}

impl<T: ?Sized> Drop for ReaderGuard<'_, T> {
    fn drop(&mut self) {
        let mut state = self.handle.state.lock();
        if let Some(ptr) = state.inner {
            while state.count == 1 {
                let to_run = {
                    // SAFETY: `state.inner` remains valid under lock.
                    let inner = unsafe { ptr.cast::<HooksInner<'_>>().as_ref() };
                    inner.bin.take_cleanups()
                };
                if to_run.is_empty() {
                    break;
                }
                drop(state);

                for cleanup in to_run {
                    cleanup();
                }

                state = self.handle.state.lock();
            }
        }
        state.count -= 1;
        if state.count == 0 {
            self.handle.condvar.notify_all();
        }
    }
}

#[derive(Default)]
struct HooksInner<'a> {
    bin: GarbageBin<'a>,
    table: HooksTable<'a>,
}

#[derive(Default)]
struct HooksTable<'a> {
    pre_commit: Hook<PreCommitHookFn<'a>>,
    before_commit: Hook<SyncHookFn<'a>>,
    unlock_resources_acquired: Hook<SyncHookFn<'a>>,
}

struct Hook<T: ?Sized> {
    ptr: Option<NonNull<T>>,
}

impl<T: ?Sized> Default for Hook<T> {
    fn default() -> Self {
        Self { ptr: None }
    }
}

// SAFETY: `Hook` wraps an `Option<NonNull<T>>` protecting a `Send + Sync` closure. Access is
// synchronized under `HooksHandle`'s state lock.
unsafe impl<T: ?Sized + Send + Sync> Send for Hook<T> {}
// SAFETY: `Hook` wraps an `Option<NonNull<T>>` protecting a `Send + Sync` closure. Access is
// synchronized under `HooksHandle`'s state lock.
unsafe impl<T: ?Sized + Send + Sync> Sync for Hook<T> {}

impl<T: ?Sized + Send + Sync> Hook<T> {
    /// # Safety
    ///
    /// The caller must guarantee that `set` is called under `handle.state.lock()`.
    unsafe fn set<'a>(&mut self, bin: &GarbageBin<'a>, reader_count: u64, hook: Box<T>)
    where
        T: 'a,
    {
        let new_hook = Hook { ptr: NonNull::new(Box::into_raw(hook)) };
        let old_hook = std::mem::replace(self, new_hook);
        if old_hook.ptr.is_some() {
            bin.add_cleanup(
                reader_count,
                Box::new(move || {
                    drop(old_hook);
                }),
            );
        }
    }

    fn get(&self) -> Option<NonNull<T>> {
        self.ptr
    }
}

impl<T: ?Sized> Drop for Hook<T> {
    fn drop(&mut self) {
        if let Some(ptr) = self.ptr.take() {
            // SAFETY: `Drop::drop` has exclusive access (`&mut self`) to `Hook`.
            let _ = unsafe { Box::from_raw(ptr.as_ptr()) };
        }
    }
}

#[derive(Default)]
struct GarbageBin<'a> {
    cleanups: Mutex<Vec<CleanupFn<'a>>>,
}

impl<'a> GarbageBin<'a> {
    fn add_cleanup(&self, reader_count: u64, cleanup: CleanupFn<'a>) {
        if reader_count == 0 {
            cleanup();
        } else {
            self.cleanups.lock().push(cleanup);
        }
    }

    fn take_cleanups(&self) -> Vec<CleanupFn<'a>> {
        std::mem::take(&mut *self.cleanups.lock())
    }
}
