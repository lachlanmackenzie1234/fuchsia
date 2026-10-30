// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use super::page::VmPagePtr;
use anonymous_page_request_bindings as bindings;
use core::pin::Pin;
use pin_init::pin_data;
use zr::{Opaque, pin_init_ffi, unsafe_pinned_drop_ffi};
use zx_status::Status;

/// Helper around tracking and performing waits for the PMM to be able to succeed waitable
/// allocations. This is intended to a have a similar Wait method as a regular PageRequest to
/// provide a consistent interface. Users are unlikely to want to use this directly, and instead
/// probably want a MultiPageRequest.
/// This class is not thread safe.
#[pin_data(PinnedDrop)]
#[repr(transparent)]
pub struct AnonymousPageRequest {
    /// The C++ object owns a possibly allocated page that its destructor must free, so this object
    /// must be pinned and dropped in place.
    #[pin]
    opaque: Opaque<bindings::AnonymousPageRequest>,
}

unsafe_pinned_drop_ffi!(AnonymousPageRequest, bindings::cpp_anonymous_page_request_destroy);

impl AnonymousPageRequest {
    /// Returns an in-place initializer for stack-pinning an `AnonymousPageRequest`.
    pub fn new() -> impl pin_init::PinInit<Self> {
        pin_init_ffi!(bindings::cpp_anonymous_page_request_construct)
    }

    /// Make the request inactive, if it was currently active. As this class is not thread safe, it
    /// assumes there are no parallel calls to Wait that would need to be interrupted.
    pub fn cancel(self: Pin<&mut Self>) {
        // SAFETY: Calling C++ Cancel on the pinned instance does not move it.
        unsafe { bindings::cpp_anonymous_page_request_cancel(self.as_raw()) }
    }

    /// Returns a raw pointer to the underlying C++ `AnonymousPageRequest`.
    ///
    /// Callers must not use the returned raw pointer to move the object in memory.
    pub fn as_raw(self: Pin<&mut Self>) -> *mut bindings::AnonymousPageRequest {
        // SAFETY: Obtaining a raw pointer to `opaque` does not move the pinned object.
        unsafe { self.get_unchecked_mut().opaque.get() }
    }

    /// Make the request active, causing any future `wait` calls to block on the PMM.
    /// Caller must make sure no page is allocated (`!has_page()`) before calling `make_active()`.
    #[inline]
    pub fn make_active(self: Pin<&mut Self>) {
        // SAFETY: `self.as_raw()` returns a valid, pinned `AnonymousPageRequest` pointer.
        unsafe { bindings::cpp_anonymous_page_request_make_active(self.as_raw()) }
    }

    /// Waits for a page to be allocated from the PMM. On success, a page is allocated.
    ///
    /// This method blocks until the PMM can fulfill the allocation and must be called with no
    /// locks held (`lockdep::AssertNoLocksHeld()`). The request must currently be active and must
    /// not already have a page allocated.
    #[inline]
    pub fn allocate(self: Pin<&mut Self>, suspendable: bool) -> Result<(), Status> {
        // SAFETY: `self.as_raw()` returns a valid, pinned pointer.
        Status::ok(unsafe {
            bindings::cpp_anonymous_page_request_allocate(self.as_raw(), suspendable)
        })
    }

    /// Returns whether a page is allocated.
    #[inline]
    pub fn has_page(&self) -> bool {
        // SAFETY: `self.opaque.get()` returns a valid `AnonymousPageRequest` pointer.
        unsafe { bindings::cpp_anonymous_page_request_has_page(self.opaque.get()) }
    }

    /// Retrieves a possibly allocated `page`.
    #[inline]
    pub fn take_page(self: Pin<&mut Self>) -> Option<VmPagePtr> {
        // SAFETY: `self.as_raw()` returns a valid, pinned pointer, and `take_page` returns either
        // null or a valid `VmPage` pointer.
        unsafe {
            let raw = bindings::cpp_anonymous_page_request_take_page(self.as_raw());
            VmPagePtr::from_ffi(raw)
        }
    }
}
