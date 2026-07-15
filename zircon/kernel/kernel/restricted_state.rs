// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use core::ffi::c_void;
use core::ptr::{self, NonNull};
use kalloc::Box;
use ltrace::ltracef;
use zx_status::Status;
use zx_types::{zx_exception_report_t, zx_restricted_state_t, zx_status_t};

use crate::arch::ArchSavedNormalState;

const LOCAL_TRACE: u32 = 0;

unsafe extern "C" {
    fn cpp_restricted_state_create_vmo_mapping(
        out_vmo: *mut *mut c_void,
        out_mapping: *mut *mut c_void,
        out_base: *mut *mut zx_restricted_state_t,
    ) -> zx_status_t;
    fn cpp_restricted_state_destroy_vmo_mapping(vmo: *mut c_void, mapping: *mut c_void);
}

/// Encapsulates a thread's restricted mode state, including VMO backing and mapping.
///
/// The memory layout of this struct (`#[repr(C)]`) must exactly match its C++ counterpart
/// (`class RestrictedState` in `zircon/kernel/include/kernel/restricted_state.h`), as instances
/// allocated here are directly accessed from C++ code by pointer. Any changes to field types,
/// names, ordering, or layout must be kept in sync between the two definitions, and verified via
/// static assertions.
#[repr(C)]
pub struct RestrictedState {
    in_restricted: bool,
    vector_ptr: usize,
    context: usize,
    exception_report_ptr: *mut zx_exception_report_t,
    vmo: NonNull<c_void>,
    mapping: NonNull<c_void>,
    state_mapping_ptr: NonNull<zx_restricted_state_t>,
    arch: ArchSavedNormalState,
}

// Verify that the memory layout of Rust `RestrictedState` exactly matches the C++
// `class RestrictedState` in `zircon/kernel/include/kernel/restricted_state.h`.
//
// Both types are accessed directly by pointer across the FFI boundary, so any changes to field
// types, ordering, or layout here must also be mirrored on the C++ side, and vice versa.
const _: () = {
    assert!(core::mem::offset_of!(RestrictedState, in_restricted) == 0);
    assert!(core::mem::offset_of!(RestrictedState, vector_ptr) == 8);
    assert!(core::mem::offset_of!(RestrictedState, context) == 16);
    assert!(core::mem::offset_of!(RestrictedState, exception_report_ptr) == 24);
    assert!(core::mem::offset_of!(RestrictedState, vmo) == 32);
    assert!(core::mem::offset_of!(RestrictedState, mapping) == 40);
    assert!(core::mem::offset_of!(RestrictedState, state_mapping_ptr) == 48);
    assert!(core::mem::offset_of!(RestrictedState, arch) == 56);
    assert!(core::mem::align_of::<RestrictedState>() == 8);
    #[cfg(not(target_arch = "riscv64"))]
    assert!(core::mem::size_of::<RestrictedState>() == 72);
    #[cfg(target_arch = "riscv64")]
    assert!(core::mem::size_of::<RestrictedState>() == 64);
};

impl RestrictedState {
    /// Create a new `RestrictedState`, allocating its VMO and mapping in kernel memory.
    pub fn create(exception_report_ptr: *mut zx_exception_report_t) -> Result<Box<Self>, Status> {
        let mut raw_vmo = ptr::null_mut();
        let mut raw_mapping = ptr::null_mut();
        let mut raw_base = ptr::null_mut();
        // Allocate VMO, pin pages, map into kernel address space, and eagerly fault in mapping.
        // SAFETY: We pass valid pointers to out variables for VMO, mapping, and base address.
        let raw_status = unsafe {
            cpp_restricted_state_create_vmo_mapping(&mut raw_vmo, &mut raw_mapping, &mut raw_base)
        };
        Status::ok(raw_status)?;

        let (vmo, mapping, state_mapping_ptr) =
            match (NonNull::new(raw_vmo), NonNull::new(raw_mapping), NonNull::new(raw_base)) {
                (Some(vmo), Some(mapping), Some(base)) => (vmo, mapping, base),
                _ => {
                    // SAFETY: raw_vmo and raw_mapping were created by cpp_restricted_state_create_vmo_mapping.
                    unsafe { cpp_restricted_state_destroy_vmo_mapping(raw_vmo, raw_mapping) };
                    return Err(Status::NO_MEMORY);
                }
            };

        ltracef!("mapping at {:#x}\n", state_mapping_ptr.as_ptr() as usize);

        let state = Box::try_new(Self {
            in_restricted: false,
            vector_ptr: 0,
            context: 0,
            exception_report_ptr,
            vmo,
            mapping,
            state_mapping_ptr,
            arch: ArchSavedNormalState::default(),
        });

        match state {
            Ok(s) => Ok(s),
            Err(_) => {
                // SAFETY: raw_vmo and raw_mapping were created by cpp_restricted_state_create_vmo_mapping.
                unsafe { cpp_restricted_state_destroy_vmo_mapping(raw_vmo, raw_mapping) };
                Err(Status::NO_MEMORY)
            }
        }
    }

    /// Return whether the thread is currently in restricted mode.
    pub fn in_restricted(&self) -> bool {
        self.in_restricted
    }

    /// Set whether the thread is currently in restricted mode.
    pub fn set_in_restricted(&mut self, val: bool) {
        self.in_restricted = val;
    }

    /// Return the normal mode vector table pointer.
    pub fn vector_ptr(&self) -> usize {
        self.vector_ptr
    }

    /// Set the normal mode vector table pointer.
    pub fn set_vector_ptr(&mut self, val: usize) {
        self.vector_ptr = val;
    }

    /// Return the normal mode context value.
    pub fn context(&self) -> usize {
        self.context
    }

    /// Set the normal mode context value.
    pub fn set_context(&mut self, val: usize) {
        self.context = val;
    }

    /// Return the user exception report pointer address.
    pub fn exception_report_ptr(&self) -> *mut zx_exception_report_t {
        self.exception_report_ptr
    }

    /// Return a shared reference to the saved normal mode architecture state.
    pub fn arch_normal_state(&self) -> &ArchSavedNormalState {
        &self.arch
    }

    /// Return a mutable reference to the saved normal mode architecture state.
    pub fn arch_normal_state_mut(&mut self) -> &mut ArchSavedNormalState {
        &mut self.arch
    }

    /// Return a shared reference to the mapped `zx_restricted_state_t` buffer.
    pub fn state(&self) -> &zx_restricted_state_t {
        // SAFETY: By RestrictedState invariant, state_mapping_ptr points to a valid zx_restricted_state_t.
        unsafe { self.state_mapping_ptr.as_ref() }
    }

    /// Return a mutable reference to the mapped `zx_restricted_state_t` buffer.
    pub fn state_mut(&mut self) -> &mut zx_restricted_state_t {
        // SAFETY: By RestrictedState invariant, state_mapping_ptr points to a valid zx_restricted_state_t.
        unsafe { self.state_mapping_ptr.as_mut() }
    }

    /// Return a raw pointer to the mapped `zx_restricted_state_t` buffer.
    pub fn state_ptr(&self) -> *mut zx_restricted_state_t {
        self.state_mapping_ptr.as_ptr()
    }

    /// Return a raw pointer to the underlying VMO.
    pub fn vmo(&self) -> *mut c_void {
        self.vmo.as_ptr()
    }
}

impl Drop for RestrictedState {
    fn drop(&mut self) {
        // Destroy the kernel mapping and unpin the VMO pages.
        // SAFETY: self.vmo and self.mapping contain valid pointers returned during creation.
        unsafe {
            cpp_restricted_state_destroy_vmo_mapping(self.vmo.as_ptr(), self.mapping.as_ptr());
        }
    }
}

/// Create a new `RestrictedState` and write its raw pointer into `out_ptr`.
///
/// # Safety
///
/// Caller must ensure `out_ptr` is a valid pointer for writing `*mut RestrictedState`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_restricted_state_create(
    exception_report_ptr: *mut zx_exception_report_t,
    out_ptr: *mut *mut RestrictedState,
) -> zx_status_t {
    match RestrictedState::create(exception_report_ptr) {
        Ok(box_state) => {
            // SAFETY: out_ptr is guaranteed by caller to be valid for writing.
            unsafe {
                *out_ptr = Box::into_raw(box_state);
            }
            Status::OK.into_raw()
        }
        Err(status) => status.into_raw(),
    }
}

/// Destroy a `RestrictedState` created by `rust_restricted_state_create`.
///
/// # Safety
///
/// Caller must pass a pointer returned by `rust_restricted_state_create` exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_restricted_state_destroy(ptr: *mut RestrictedState) {
    if !ptr.is_null() {
        // SAFETY: ptr was created by Box::into_raw in rust_restricted_state_create and passed once.
        unsafe {
            let _ = Box::from_raw(ptr);
        }
    }
}
