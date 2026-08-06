// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use crate::vm::page::VmPagePtr;
use core::marker::{PhantomData, PhantomPinned};
use page_queues_bindings as bindings;

#[repr(C)]
pub struct PageQueues {
    raw: bindings::PageQueues,
    phantom: PhantomData<PhantomPinned>,
}

impl PageQueues {
    /// Domain-specific conversion: returns raw pointer for `PageQueues`.
    pub fn as_raw(&self) -> *mut bindings::PageQueues {
        core::ptr::from_ref(&self.raw).cast_mut()
    }

    /// Returns whether `page` is in the wired queue.
    ///
    /// # Safety
    ///
    /// The caller must ensure `page` is a valid page pointer.
    pub unsafe fn debug_page_is_wired(&self, page: VmPagePtr) -> bool {
        // SAFETY: The caller guarantees via function safety preconditions that `page` is a valid
        // page pointer.
        unsafe { bindings::cpp_page_queues_debug_page_is_wired(self.as_raw(), page.as_raw()) }
    }

    /// Returns whether `page` is in any anonymous queue.
    ///
    /// # Safety
    ///
    /// The caller must ensure `page` is a valid page pointer.
    pub unsafe fn debug_page_is_any_anonymous(&self, page: VmPagePtr) -> bool {
        // SAFETY: The caller guarantees via function safety preconditions that `page` is a valid
        // page pointer.
        unsafe {
            bindings::cpp_page_queues_debug_page_is_any_anonymous(self.as_raw(), page.as_raw())
        }
    }
}
