// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use core::ops::Deref;
use core::ptr::NonNull;
use fbl::{IsOpaqueRefCounted, OpaqueRefCountedFacade, RefPtr};
use kernel::types::PAddr;
use ksync::{KMutex, RawCriticalMutex, guarded};
use zr::ToMutPtr;
use zx_status::Status;

use super::vm_object::VmObject;
use vm_constants_rs as constants;

// Assert size and alignment of VmObjectPhysicalState matches the generated C++ constants.
::zr::static_assert_size_and_align!(
    VmObjectPhysicalState,
    constants::kVmObjectPhysicalStateSize,
    constants::kVmObjectPhysicalStateAlign,
);

#[guarded]
#[repr(C)]
pub struct VmObjectPhysicalState {
    #[mutex]
    lock: KMutex<RawCriticalMutex>,

    size: u64,
    base: PAddr,
    is_slice: bool,
    parent_user_id: u64,

    // parent is guarded by ChildListLock on the C++ side.
    parent: Option<RefPtr<VmObjectPhysical>>,
}

impl VmObjectPhysicalState {
    pub fn init(
        base: PAddr,
        size: u64,
        is_slice: bool,
        parent_user_id: u64,
    ) -> impl pin_init::PinInit<Self, core::convert::Infallible> {
        pin_init::pin_init!(Self {
            lock <- KMutex::init(),
            size,
            base,
            is_slice,
            parent_user_id,
            parent: None,
        })
    }
}

#[repr(C)]
/// VMO representing a physical range of memory
pub struct VmObjectPhysical {
    _facade: OpaqueRefCountedFacade<VmObject>,
}

unsafe impl IsOpaqueRefCounted for VmObjectPhysical {
    type TargetBase = VmObject;
}

impl Deref for VmObjectPhysical {
    type Target = VmObject;
    fn deref(&self) -> &Self::Target {
        // SAFETY: `raw` is derived from the valid reference `self`. The FFI helper performs
        // a safe `static_cast` to the base `VmObject`, returning a valid pointer that is safe
        // to dereference for the lifetime of `self`.
        unsafe {
            let raw = self as *const Self as *mut Self;
            &*cpp_vm_object_physical_as_vm_object(raw)
        }
    }
}

unsafe extern "C" {
    fn cpp_vm_object_physical_create(
        base: PAddr,
        size: usize,
        out_status: *mut i32,
    ) -> *mut VmObjectPhysical;
    fn cpp_vm_object_physical_as_vm_object(vmo: *mut VmObjectPhysical) -> *mut VmObject;
}

impl VmObjectPhysical {
    /// Create a new physical VMO for the given physical region.
    pub fn create(base: PAddr, size: usize) -> Result<RefPtr<VmObjectPhysical>, Status> {
        let mut status = 0;
        // SAFETY: The pointer derived from `&mut status` is valid for writing an i32.
        let raw = unsafe { cpp_vm_object_physical_create(base, size, &mut status) };
        Status::ok(status)?;
        // SAFETY: The raw pointer returned by C++ is refcounted and ownership is transferred
        // to Rust via RefPtr.
        unsafe { RefPtr::try_from_raw(raw).ok_or(Status::NO_MEMORY) }
    }

    /// Cast a pointer to a VmObjectPhysical to its base VmObject.
    pub fn cast(vmo: NonNull<VmObjectPhysical>) -> NonNull<VmObject> {
        // SAFETY: Calls C++ helper to cast VmObjectPhysical pointer to base class VmObject.
        // The return value is guaranteed to be non-null and valid.
        unsafe { NonNull::new_unchecked(cpp_vm_object_physical_as_vm_object(vmo.as_ptr())) }
    }
}

// FFI trampolines for C++ calling into Rust VmObjectPhysicalState

/// # Safety
///
/// The caller must ensure `ptr` points to uninitialized memory of at least
/// `size_of::<VmObjectPhysicalState>()` bytes with proper alignment.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_vm_object_physical_state_init(
    ptr: *mut VmObjectPhysicalState,
    base: PAddr,
    size: u64,
    is_slice: bool,
    parent_user_id: u64,
) {
    // SAFETY: The caller guarantees `ptr` points to uninitialized memory of at least
    // `size_of::<VmObjectPhysicalState>()` bytes with proper alignment.
    unsafe {
        let _ = pin_init::PinInit::__pinned_init(
            VmObjectPhysicalState::init(base, size, is_slice, parent_user_id),
            ptr,
        );
    }
}

/// # Safety
///
/// The caller must ensure `ptr` points to an initialized `VmObjectPhysicalState`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_vm_object_physical_state_destroy(ptr: *mut VmObjectPhysicalState) {
    // SAFETY: The caller guarantees `ptr` points to an initialized `VmObjectPhysicalState`.
    unsafe {
        core::ptr::drop_in_place(ptr);
    }
}

/// # Safety
///
/// The caller must ensure `ptr` points to an initialized `VmObjectPhysicalState`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_vm_object_physical_state_get_lock(
    ptr: *const VmObjectPhysicalState,
) -> *mut KMutex<VmObjectPhysicalStateLockClass, RawCriticalMutex> {
    // SAFETY: The caller guarantees `ptr` points to an initialized `VmObjectPhysicalState`.
    unsafe {
        let lock_ref = &(*ptr).lock;
        lock_ref.to_mut_ptr()
    }
}

/// # Safety
///
/// The caller must ensure `ptr` points to an initialized `VmObjectPhysicalState`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_vm_object_physical_state_get_size(
    ptr: *const VmObjectPhysicalState,
) -> u64 {
    // SAFETY: The caller guarantees `ptr` points to an initialized `VmObjectPhysicalState`.
    unsafe { (*ptr).size }
}

/// # Safety
///
/// The caller must ensure `ptr` points to an initialized `VmObjectPhysicalState`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_vm_object_physical_state_get_base(
    ptr: *const VmObjectPhysicalState,
) -> PAddr {
    // SAFETY: The caller guarantees `ptr` points to an initialized `VmObjectPhysicalState`.
    unsafe { (*ptr).base }
}

/// # Safety
///
/// The caller must ensure `ptr` points to an initialized `VmObjectPhysicalState`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_vm_object_physical_state_get_is_slice(
    ptr: *const VmObjectPhysicalState,
) -> bool {
    // SAFETY: The caller guarantees `ptr` points to an initialized `VmObjectPhysicalState`.
    unsafe { (*ptr).is_slice }
}

/// # Safety
///
/// The caller must ensure `ptr` points to an initialized `VmObjectPhysicalState`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_vm_object_physical_state_get_parent_user_id(
    ptr: *const VmObjectPhysicalState,
) -> u64 {
    // SAFETY: The caller guarantees `ptr` points to an initialized `VmObjectPhysicalState`.
    unsafe { (*ptr).parent_user_id }
}

/// # Safety
///
/// The caller must ensure `ptr` points to an initialized `VmObjectPhysicalState`.
/// The caller must ensure that the `ChildListLock` is held.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_vm_object_physical_state_get_parent_locked(
    ptr: *const VmObjectPhysicalState,
) -> *mut VmObjectPhysical {
    // SAFETY: The caller guarantees `ptr` points to an initialized `VmObjectPhysicalState`
    // and that the ChildListLock is held.
    unsafe {
        match &(*ptr).parent {
            Some(ref_ptr) => RefPtr::into_raw(ref_ptr.clone()).cast_mut(),
            None => core::ptr::null_mut(),
        }
    }
}

/// # Safety
///
/// The caller must ensure `ptr` points to an initialized `VmObjectPhysicalState`.
/// The caller must ensure that the `ChildListLock` is held.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_vm_object_physical_state_set_parent_locked(
    ptr: *mut VmObjectPhysicalState,
    parent: *mut VmObjectPhysical,
) {
    // SAFETY: The caller guarantees `ptr` points to an initialized `VmObjectPhysicalState`
    // and that the ChildListLock is held.
    unsafe {
        let parent_ref = if parent.is_null() { None } else { Some(RefPtr::from_raw(parent)) };
        (*ptr).parent = parent_ref;
    }
}
