// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use super::dispatcher::{Dispatcher, DispatcherOps};
use super::handle::{HandleOwner, HandleRef, HandleValue};
use super::handle_table_ffi::{
    cpp_handle_table_add_handle_locked, cpp_handle_table_get_handle_locked, cpp_handle_table_koid,
    cpp_handle_table_lock, cpp_handle_table_map_handle_to_value,
    cpp_handle_table_remove_handle_locked,
};
use super::process_dispatcher::ProcessDispatcher;
use crate::kernel::thread::AutoExpiringPreemptDisabler;
use core::convert::Infallible;
use core::ffi::c_void;
use core::fmt::{self, Debug, Formatter};
use core::mem::size_of;
use core::ptr::{self, NonNull};
use fbl::{HasRefCount, Recyclable, RefPtr};
use ksync::{BrwLockPi, BrwLockPiReadGuard, BrwLockPiWriteGuard, LockClass};
use pin_init::{PinInit, pin_data, pin_init};
use zr::OpaqueFacade;
use zx_status::Status;
use zx_types::{ZX_HANDLE_INVALID, ZX_OBJ_TYPE_NONE, zx_handle_t, zx_koid_t, zx_rights_t};

/// Lock class tag for the handle table's reader-writer lock.
#[derive(Debug, Default)]
struct HandleTableLockClass;

impl LockClass for HandleTableLockClass {
    const ID: *mut c_void = ptr::null_mut();
}

/// Facade for the C++ `HandleTable` class.
#[repr(C)]
pub struct HandleTable {
    _facade: OpaqueFacade,
}

zr::static_assert!(size_of::<HandleTable>() == 0);

impl Debug for HandleTable {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("HandleTable").field("koid", &self.get_koid()).finish()
    }
}

impl HandleTable {
    /// Returns a `*const HandleTable` pointer suitable for passing to FFI routines.
    #[inline]
    pub fn as_ffi(&self) -> *const Self {
        self as *const Self
    }

    /// Returns a `*mut HandleTable` pointer suitable for passing to FFI routines that
    /// take a non-const `HandleTable*`.
    #[inline]
    pub fn as_ffi_mut(&self) -> *mut Self {
        self.as_ffi() as *mut Self
    }

    /// Returns a reference to the handle table's priority-inheriting reader-writer lock.
    #[inline]
    fn lock(&self) -> &BrwLockPi<HandleTableLockClass> {
        // SAFETY: `self` is a valid `HandleTable`, and its lock is a valid `BrwLockPi`.
        unsafe {
            let lock_ptr = cpp_handle_table_lock(self.as_ffi());
            &*(lock_ptr as *const BrwLockPi<HandleTableLockClass>)
        }
    }

    /// Acquires a reader lock on this handle table.
    #[inline]
    pub fn read_lock(&self) -> impl PinInit<HandleTableReadGuard<'_>, Infallible> {
        HandleTableReadGuard::new(self)
    }

    /// Acquires a writer lock on this handle table.
    #[inline]
    pub fn write_lock(&self) -> impl PinInit<HandleTableWriteGuard<'_>, Infallible> {
        HandleTableWriteGuard::new(self)
    }

    /// Returns the KOID of this handle table.
    #[inline]
    pub fn get_koid(&self) -> zx_koid_t {
        // SAFETY: `self` is a valid `HandleTable` reference.
        unsafe { cpp_handle_table_koid(self.as_ffi()) }
    }

    /// Maps a handle to its user-visible `HandleValue` for this handle table.
    #[inline]
    pub fn map_handle_to_value(&self, handle: HandleRef<'_>) -> HandleValue {
        // SAFETY: `self` is a valid `HandleTable` and `handle` is a valid `HandleRef`.
        let raw = unsafe { cpp_handle_table_map_handle_to_value(self.as_ffi(), handle.as_ptr()) };
        HandleValue::new(raw)
    }

    /// Removes a handle from this handle table and returns the owned handle if found.
    pub fn remove_handle(
        &self,
        caller: &ProcessDispatcher,
        handle: HandleValue,
    ) -> Option<HandleOwner> {
        let _preempt_disable = AutoExpiringPreemptDisabler::with_default_timeslice_extension();
        ksync::lock!(let guard = self.write_lock());
        guard.remove_handle(caller, handle)
    }

    /// Removes a slice of raw handle values from this handle table.
    ///
    /// Matching C++ `HandleTable::RemoveHandles(ProcessDispatcher&, ktl::span<const zx_handle_t>)`,
    /// `ZX_HANDLE_INVALID` entries are skipped, and if any non-zero handle is missing, returns
    /// `ZX_ERR_BAD_HANDLE` after processing the remaining handles.
    pub fn remove_handles(
        &self,
        caller: &ProcessDispatcher,
        handles: &[zx_handle_t],
    ) -> Result<(), Status> {
        let mut status = Ok(());
        let _preempt_disable = AutoExpiringPreemptDisabler::with_default_timeslice_extension();
        ksync::lock!(let guard = self.write_lock());
        for &handle in handles {
            if handle != ZX_HANDLE_INVALID
                && guard.remove_handle(caller, HandleValue::new(handle)).is_none()
            {
                status = Err(Status::BAD_HANDLE);
            }
        }
        status
    }

    /// Resolves a handle to a generic dispatcher and returns its associated rights.
    #[inline]
    pub fn get_dispatcher_and_rights(
        &self,
        caller: &ProcessDispatcher,
        handle_value: HandleValue,
    ) -> Result<(RefPtr<Dispatcher>, zx_rights_t), Status> {
        ksync::lock!(let guard = self.read_lock());
        let handle = guard.get_handle(caller, handle_value).ok_or(Status::BAD_HANDLE)?;
        Ok((handle.dispatcher(), handle.rights()))
    }

    /// Resolves a handle to a dispatcher of type `T` with the required `rights` in this handle
    /// table.
    #[inline]
    pub fn get_dispatcher_with_rights<T>(
        &self,
        caller: &ProcessDispatcher,
        handle_value: HandleValue,
        rights: zx_rights_t,
    ) -> Result<RefPtr<T>, Status>
    where
        T: DispatcherOps + HasRefCount + Recyclable,
    {
        const { assert!(T::TYPE != ZX_OBJ_TYPE_NONE) };
        let (dispatcher, actual_rights) = self.get_dispatcher_and_rights(caller, handle_value)?;
        if dispatcher.get_type() != T::TYPE {
            return Err(Status::WRONG_TYPE);
        }
        if (actual_rights & rights) != rights {
            return Err(Status::ACCESS_DENIED);
        }
        // SAFETY: We verified the type of the dispatcher matches `T::TYPE`, so it is safe to cast.
        Ok(unsafe { dispatcher.cast::<T>() })
    }
}

/// RAII reader lock guard for a handle table.
///
/// Encapsulates the reader lock on the handle table, ensuring that handle lookups and rights
/// checks can only occur while the lock is held.
#[pin_data]
pub struct HandleTableReadGuard<'a> {
    handle_table: &'a HandleTable,
    #[pin]
    guard: BrwLockPiReadGuard<'a, HandleTableLockClass>,
}

impl<'a> HandleTableReadGuard<'a> {
    /// Creates a stack-pinned handle table reader lock guard for `handle_table`.
    pub fn new(handle_table: &'a HandleTable) -> impl PinInit<Self, Infallible> {
        pin_init!(Self {
            handle_table,
            guard <- handle_table.lock().read_lock(),
        })
    }

    /// Retrieves a handle reference while holding the handle table lock.
    pub fn get_handle(
        &self,
        caller: &ProcessDispatcher,
        handle_value: HandleValue,
    ) -> Option<HandleRef<'_>> {
        // SAFETY: `self.handle_table` and `caller` are valid and the handle table lock is held for
        // the duration of `self`.
        let ptr = unsafe {
            cpp_handle_table_get_handle_locked(
                self.handle_table.as_ffi_mut(),
                caller.as_ffi_mut(),
                handle_value.raw_value(),
            )
        };
        // SAFETY: `ptr` is a valid handle pointer in `self.handle_table` while the lock is held.
        NonNull::new(ptr).map(|ptr| unsafe { HandleRef::from_raw(ptr) })
    }
}

/// RAII writer lock guard for a handle table.
///
/// Encapsulates the writer lock on the handle table, allowing handle lookups, additions, and
/// removals while the lock is held.
#[pin_data]
pub struct HandleTableWriteGuard<'a> {
    handle_table: &'a HandleTable,
    #[pin]
    guard: BrwLockPiWriteGuard<'a, HandleTableLockClass>,
}

impl<'a> HandleTableWriteGuard<'a> {
    /// Creates a stack-pinned handle table writer lock guard for `handle_table`.
    pub fn new(handle_table: &'a HandleTable) -> impl PinInit<Self, Infallible> {
        pin_init!(Self {
            handle_table,
            guard <- handle_table.lock().write_lock(),
        })
    }

    /// Retrieves a handle reference while holding the handle table write lock.
    #[inline]
    pub fn get_handle(
        &self,
        caller: &ProcessDispatcher,
        handle_value: HandleValue,
    ) -> Option<HandleRef<'_>> {
        // SAFETY: `self.handle_table` and `caller` are valid and the handle table write lock is
        // held for the duration of `self`.
        let ptr = unsafe {
            cpp_handle_table_get_handle_locked(
                self.handle_table.as_ffi_mut(),
                caller.as_ffi_mut(),
                handle_value.raw_value(),
            )
        };
        // SAFETY: `ptr` is a valid handle pointer in `self.handle_table` while the lock is held.
        NonNull::new(ptr).map(|ptr| unsafe { HandleRef::from_raw(ptr) })
    }

    /// Adds an owned handle to the handle table while holding the write lock.
    #[inline]
    pub fn add_handle(&self, handle: HandleOwner) {
        // SAFETY: `self.handle_table` is valid, the handle table write lock is held, and `handle`
        // ownership is transferred to C++.
        unsafe {
            cpp_handle_table_add_handle_locked(self.handle_table.as_ffi_mut(), handle.release());
        }
    }

    /// Removes a handle by value from the handle table while holding the write lock.
    #[inline]
    pub fn remove_handle(
        &self,
        caller: &ProcessDispatcher,
        handle_value: HandleValue,
    ) -> Option<HandleOwner> {
        self.get_handle(caller, handle_value).map(|handle| self.remove_handle_ref(handle))
    }

    /// Removes a handle known to be in this handle table while holding the write lock.
    #[inline]
    pub fn remove_handle_ref(&self, handle: HandleRef<'_>) -> HandleOwner {
        // SAFETY: `self.handle_table` is valid, the handle table write lock is held, and `handle`
        // was looked up in `self.handle_table` under the same lock.
        let raw = unsafe {
            cpp_handle_table_remove_handle_locked(
                self.handle_table.as_ffi_mut(),
                handle.as_ptr() as *mut _,
            )
        };
        // SAFETY: `raw` is a non-null owned handle pointer released from the handle table.
        unsafe { HandleOwner::from_raw(raw).unwrap() }
    }
}
