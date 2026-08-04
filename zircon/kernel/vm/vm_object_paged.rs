// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use super::page::VmPagePtr;
use super::vm_cow_pages::VmCowPages;
use super::vm_object::VmObject;
use core::marker::PhantomPinned;
use core::ops::Deref;
use fbl::{IsOpaqueRefCounted, RefPtr};
use vm_object_paged_bindings as bindings;
use zx_status::Status;

/// VMO representing a paged range of copy-on-write memory.
#[repr(C)]
pub struct VmObjectPaged {
    raw: bindings::VmObjectPaged,
    phantom: PhantomPinned,
}

impl VmObjectPaged {
    pub const ALWAYS_PINNED: u32 = bindings::VmObjectPaged_kAlwaysPinned;

    /// Domain-specific conversion: returns raw FFI pointer for `VmObjectPaged`.
    pub fn as_raw(&self) -> *mut bindings::VmObjectPaged {
        core::ptr::from_ref(&self.raw).cast_mut()
    }

    /// Domain-specific conversion: constructs a `RefPtr<VmObjectPaged>` from a raw FFI pointer.
    ///
    /// # Safety
    ///
    /// `ptr` must be a valid, raw `VmObjectPaged` pointer exported from C++.
    pub unsafe fn from_raw(ptr: *mut bindings::VmObjectPaged) -> Option<RefPtr<Self>> {
        unsafe { RefPtr::try_from_raw(ptr.cast::<Self>()) }
    }

    /// Create a new paged VMO.
    pub fn create(
        pmm_alloc_flags: u32,
        options: u32,
        size: u64,
    ) -> Result<RefPtr<VmObjectPaged>, Status> {
        let mut status = 0;
        let raw = unsafe {
            bindings::cpp_vm_object_paged_create(pmm_alloc_flags, options, size, &mut status)
        };
        Status::ok(status)?;
        unsafe { Self::from_raw(raw).ok_or(Status::NO_MEMORY) }
    }

    /// Create a contiguous paged VMO.
    pub fn create_contiguous(
        pmm_alloc_flags: u32,
        size: u64,
        alignment_log2: u8,
    ) -> Result<RefPtr<VmObjectPaged>, Status> {
        let mut status = 0;
        // SAFETY: status is a valid local mutable reference.
        let raw = unsafe {
            bindings::cpp_vm_object_paged_create_contiguous(
                pmm_alloc_flags,
                size,
                alignment_log2,
                &mut status,
            )
        };
        Status::ok(status)?;
        unsafe { Self::from_raw(raw).ok_or(Status::NO_MEMORY) }
    }

    /// Returns the backing `VmCowPages` hierarchy.
    pub fn debug_get_cow_pages(&self) -> Option<RefPtr<VmCowPages>> {
        let raw = unsafe { bindings::cpp_vm_object_paged_debug_get_cow_pages(self.as_raw()) };
        unsafe { VmCowPages::from_raw(raw) }
    }

    /// Debug helper to fetch backing page pointer.
    pub fn debug_get_page(&self, offset: u64) -> Option<VmPagePtr> {
        let raw = unsafe { bindings::cpp_vm_object_paged_debug_get_page(self.as_raw(), offset) };
        unsafe { VmPagePtr::from_raw(raw) }
    }
}

unsafe impl IsOpaqueRefCounted for VmObjectPaged {
    type TargetBase = VmObject;
}

impl Deref for VmObjectPaged {
    type Target = VmObject;
    fn deref(&self) -> &Self::Target {
        let raw = unsafe { bindings::cpp_vm_object_paged_as_vm_object(self.as_raw()) };
        let ptr = VmObject::ptr_from_raw(raw);
        // SAFETY: cpp_vm_object_paged_as_vm_object returns a valid pointer with the same lifetime
        // as its input, so `raw`, and trivially `ptr`, are valid.
        unsafe { ptr.as_ref_unchecked() }
    }
}
