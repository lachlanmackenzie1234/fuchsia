// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use super::arch_vm_aspace::ArchMmuFlags;
use core::marker::{PhantomData, PhantomPinned};
use core::ptr::NonNull;
use fbl::{HasRefCount, Recyclable, RefPtr};
use kalloc::AllocError;
use vm_object_bindings as bindings;
use zx_status::Status;

pub use bindings::{Resizability, SnapshotType, VmObject_EvictionHint as EvictionHint};

/// The base vm object that holds a range of bytes of data
///
/// Can be created without mapping and used as a container of data, or mappable
/// into an address space via VmAddressRegion::CreateVmMapping
#[repr(C)]
pub struct VmObject {
    raw: bindings::VmObject,
    phantom: PhantomData<PhantomPinned>,
}

impl VmObject {
    pub const MAX_SIZE: u64 = bindings::VmObject_MAX_SIZE;

    /// Domain-specific conversion: returns raw pointer for `VmObject`.
    pub fn as_raw(&self) -> *mut bindings::VmObject {
        core::ptr::from_ref(&self.raw).cast_mut()
    }

    /// Domain-specific conversion: constructs a `RefPtr` from an exported pointer.
    ///
    /// # Safety
    ///
    /// `ptr` must be a valid raw `VmObject` pointer exported from C++.
    pub unsafe fn from_raw(ptr: *mut bindings::VmObject) -> Option<RefPtr<Self>> {
        unsafe { RefPtr::try_from_raw(ptr.cast::<Self>()) }
    }

    /// Returns a raw `VmObject` pointer from an underlying bindings pointer.
    ///
    /// Provides additional type safety when used instead of a `.cast()`.
    pub fn ptr_from_raw(raw: *mut bindings::VmObject) -> *mut VmObject {
        raw.cast()
    }

    /// Returns a pointer to the underlying `VmObject` structure.
    ///
    /// This method is helpful when you don't have a reference to the `VmObject`. If you do, then
    /// use `VmObject::as_raw` instead.
    pub fn cast_raw(ptr: *mut VmObject) -> *mut bindings::VmObject {
        ptr.cast()
    }

    /// Returns the size of the VMO in bytes.
    pub fn size(&self) -> u64 {
        // SAFETY: `self.as_raw()` returns a valid `VmObject` pointer.
        unsafe { bindings::cpp_vm_object_size(self.as_raw()) }
    }

    /// Returns whether the VMO is resizable.
    pub fn is_resizable(&self) -> bool {
        // SAFETY: `self.as_raw()` returns a valid `VmObject` pointer.
        unsafe { bindings::cpp_vm_object_is_resizable(self.as_raw()) }
    }

    /// Returns whether the VMO is contiguous.
    pub fn is_contiguous(&self) -> bool {
        // SAFETY: `self.as_raw()` returns a valid `VmObject` pointer.
        unsafe { bindings::cpp_vm_object_is_contiguous(self.as_raw()) }
    }

    /// Resizes the VMO to the given size.
    pub fn resize(&self, size: u64) -> Result<(), Status> {
        // SAFETY: `self.as_raw()` returns a valid `VmObject` pointer.
        let status = unsafe { bindings::cpp_vm_object_resize(self.as_raw(), size) };
        Status::ok(status)
    }

    /// Writes data from `data` slice into the VMO at `offset`.
    pub fn write(&self, offset: u64, data: &[u8]) -> Result<(), Status> {
        // SAFETY: `self.as_raw()` returns a valid `VmObject` pointer and `data` points to
        // `data.len()` bytes of valid memory.
        let status = unsafe {
            bindings::cpp_vm_object_write(self.as_raw(), data.as_ptr().cast(), offset, data.len())
        };
        Status::ok(status)
    }

    /// Sets the name of the VMO.
    pub fn set_name(&self, name: &[u8]) -> Result<(), Status> {
        // SAFETY: `self.as_raw()` returns a valid `VmObject` pointer and `name` points to
        // `name.len()` bytes of valid memory.
        let status = unsafe {
            bindings::cpp_vm_object_set_name(self.as_raw(), name.as_ptr().cast(), name.len())
        };
        Status::ok(status)
    }

    /// Decommit a range of pages from the VMO.
    pub fn decommit_range(&self, offset: u64, len: u64) -> Result<(), Status> {
        // SAFETY: `self.as_raw()` returns a valid `VmObject` pointer.
        let status = unsafe { bindings::cpp_vm_object_decommit_range(self.as_raw(), offset, len) };
        Status::ok(status)
    }

    /// Commits and pins the specified range of pages in the VMO.
    pub fn commit_range_pinned(&self, offset: u64, len: u64, write: bool) -> Result<(), Status> {
        // SAFETY: `self.as_raw()` returns a valid `VmObject` pointer.
        let status = unsafe {
            bindings::cpp_vm_object_commit_range_pinned(self.as_raw(), offset, len, write)
        };
        Status::ok(status)
    }

    /// Unpins the specified range of pages in the VMO.
    pub fn unpin(&self, offset: u64, len: u64) {
        // SAFETY: `self.as_raw()` returns a valid `VmObject` pointer.
        unsafe {
            bindings::cpp_vm_object_unpin(self.as_raw(), offset, len);
        }
    }

    /// Provide an eviction hint for a range of pages.
    pub fn hint_range(&self, offset: u64, len: u64, hint: EvictionHint) -> Result<(), Status> {
        let status =
            unsafe { bindings::cpp_vm_object_hint_range(self.as_raw(), offset, len, hint) };
        Status::ok(status)
    }

    /// Returns the mapping cache policy of the VMO.
    pub fn get_mapping_cache_policy(&self) -> ArchMmuFlags {
        // SAFETY: `self.as_raw()` returns a valid `VmObject` pointer.
        unsafe { bindings::cpp_vm_object_get_mapping_cache_policy(self.as_raw()) }
    }

    /// Create a copy-on-write clone VMO at the page-aligned offset and length.
    pub fn create_clone(
        &self,
        resizable: Resizability,
        snapshot_type: SnapshotType,
        offset: u64,
        size: u64,
        copy_name: bool,
    ) -> Result<RefPtr<VmObject>, Status> {
        let mut status = 0;
        // SAFETY: `self.as_raw()` returns a valid `VmObject` pointer.
        let raw = unsafe {
            bindings::cpp_vm_object_create_clone(
                self.as_raw(),
                resizable,
                snapshot_type,
                offset,
                size,
                copy_name,
                &mut status,
            )
        };
        Status::ok(status)?;
        // SAFETY: cpp_vm_object_create_clone returns valid VmObject pointers, or null.
        let clone = unsafe { VmObject::from_raw(raw) };
        Ok(clone.expect("clone returned ZX_OK; must be non-null"))
    }

    /// Helper variant of get_page that will retry the operation after waiting on a PageRequest if required.
    pub fn get_page_blocking(&self, offset: u64, pf_flags: u32) -> Result<(), Status> {
        // SAFETY: `self.as_raw()` returns a valid `VmObject` pointer.
        let status =
            unsafe { bindings::cpp_vm_object_get_page_blocking(self.as_raw(), offset, pf_flags) };
        Status::ok(status)
    }

    /// Sets the user ID of the VMO.
    pub fn set_user_id(&self, user_id: u64) {
        // SAFETY: `self.as_raw()` returns a valid `VmObject` pointer.
        unsafe { bindings::cpp_vm_object_set_user_id(self.as_raw(), user_id) }
    }

    /// Returns the user ID of the VMO.
    pub fn user_id(&self) -> u64 {
        // SAFETY: `self.as_raw()` returns a valid `VmObject` pointer.
        unsafe { bindings::cpp_vm_object_user_id(self.as_raw()) }
    }

    /// Returns the user ID of the parent VMO, if any.
    pub fn parent_user_id(&self) -> u64 {
        // SAFETY: `self.as_raw()` returns a valid `VmObject` pointer.
        unsafe { bindings::cpp_vm_object_parent_user_id(self.as_raw()) }
    }
}

impl HasRefCount for VmObject {
    fn ref_count(&self) -> &fbl::RefCounted {
        let raw = unsafe { bindings::cpp_vm_object_get_ref_counted(self.as_raw()) };
        unsafe { &*(raw.cast::<fbl::RefCounted>()) }
    }
}

unsafe impl Recyclable for VmObject {
    unsafe fn recycle(ptr: NonNull<Self>) {
        unsafe {
            bindings::cpp_vm_object_free(VmObject::cast_raw(ptr.as_ptr()));
        }
    }

    fn allocate(_value: Self) -> Result<NonNull<Self>, AllocError> {
        Err(AllocError)
    }
}
