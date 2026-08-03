// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

/// VMO tests duplicated from vmo_unittest.cc.
#[cfg(ktest)]
#[unittest::suite]
mod vmo_rs {
    use crate::vm::physical_page_borrowing_config::ScopedLoaningEnabled;
    use crate::vm::pmm::ALLOC_FLAG_ANY;
    use crate::vm::scanner::AutoVmScannerDisable;
    use crate::vm::vm_object::EvictionHint;
    use crate::vm::vm_object_paged::VmObjectPaged;
    use crate::vm_unittests::test_helper::make_committed_pager_vmo;
    use page::SIZE as PAGE_SIZE_USIZE;
    use unittest::{assert_false, assert_ok, unwrap_ok};

    const PAGE_SIZE: u64 = PAGE_SIZE_USIZE as u64;

    /// Tests HintRange(AlwaysNeed) evicts loaned pages.
    #[test]
    fn vmo_always_need_evicts_loaned_test() {
        let _scanner_disable = AutoVmScannerDisable::new();

        // Depending on which loaned page we get, it may not still be loaned at
        // the time HintRange() is called, so try a few times and make sure we
        // see non-loaned after HintRange() for all the tries.
        let try_count = 30;
        for _try_ordinal in 0..try_count {
            let _enable_loaning = ScopedLoaningEnabled::new(true);

            // create a contiguous VMO so that we are guaranteed to have a place to borrow from
            let contiguous_vmo = unwrap_ok!(VmObjectPaged::create_contiguous(
                ALLOC_FLAG_ANY,
                PAGE_SIZE,
                /*alignment_log2*/ 0
            ));
            assert_ok!(contiguous_vmo.decommit_range(0, PAGE_SIZE));

            // we will replace the only page in vmo with a loaned page
            let (vmo, [before_page]) = unwrap_ok!(make_committed_pager_vmo(
                /*trap_dirty*/ false, /*resizable*/ false
            ));
            let offset = 0;
            let cow_pages = vmo.debug_get_cow_pages().expect("paged VMO has backing cow pages");
            assert_ok!(cow_pages.replace_page_with_loaned(before_page, offset));
            // The call to ReplacePageWithLoaned may loan vmo's page to a VMO that's
            // not contiguous_vmo. So, it might get called back, and the rest of the
            // test must tolerate the vmo's page becoming unloaned at any time.

            // Hint that the page is always needed.
            assert_ok!(vmo.hint_range(0, PAGE_SIZE, EvictionHint::AlwaysNeed));

            // If the page was still loaned, it will be replaced with a non-loaned page now.
            let page =
                vmo.debug_get_page(0).expect("vmo should have a page at offset 0 after hint_range");

            assert_false!(unsafe { page.is_loaned() });
        }
    }
}
