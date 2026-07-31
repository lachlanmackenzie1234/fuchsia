// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use core::marker::{PhantomData, PhantomPinned};
use core::ptr::NonNull;
use fbl::{HasRefCount, Recyclable, RefPtr};
use kalloc::AllocError;
use vm_object_bindings as bindings;
use zx_status::Status;

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
