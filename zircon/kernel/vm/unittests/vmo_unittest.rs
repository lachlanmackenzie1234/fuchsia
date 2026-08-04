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
    use crate::vm::vm_object::{EvictionHint, VmObject};
    use crate::vm::vm_object_paged::VmObjectPaged;
    use crate::vm_unittests::test_helper::make_committed_pager_vmo;
    use page::SIZE as PAGE_SIZE_USIZE;
    use unittest::{assert_eq, assert_false, assert_ok, expect_eq, expect_ok, unwrap_ok};
    use zx_status::Status;

    const PAGE_SIZE: u64 = PAGE_SIZE_USIZE as u64;

    /// Tests creating a VMO with maximum size and larger than maximum size.
    #[test]
    fn vmo_create_maximum_size() {
        let vmo = VmObjectPaged::create(ALLOC_FLAG_ANY, 0, VmObject::MAX_SIZE);
        // should be ok
        expect_ok!(vmo.map(|_| ()));

        let vmo = VmObjectPaged::create(ALLOC_FLAG_ANY, 0, VmObject::MAX_SIZE + PAGE_SIZE);
        // should be too large
        expect_eq!(Status::result_into_raw(vmo.map(|_| ())), Status::OUT_OF_RANGE.into_raw());
    }

    /// Checks that VMOs must be page aligned sizes.
    #[test]
    fn vmo_unaligned_size_test() {
        let _scanner_disable = AutoVmScannerDisable::new();

        let alloc_size = 15;
        let result = VmObjectPaged::create(ALLOC_FLAG_ANY, 0, alloc_size);
        assert_eq!(Status::result_into_raw(result.map(|_| ())), Status::INVALID_ARGS.into_raw());
    }

    /// Tests that decommitting from a contiguous VMO fails when loaning is disabled.
    #[test]
    fn vmo_contiguous_decommit_disabled_test() {
        let _enable_loaning = ScopedLoaningEnabled::new(false);

        let alloc_size = PAGE_SIZE * 16;
        let vmo = unwrap_ok!(VmObjectPaged::create_contiguous(
            ALLOC_FLAG_ANY,
            alloc_size,
            /*alignment_log2*/ 0
        ));

        // decommit fails as expected
        assert_eq!(
            Status::result_into_raw(vmo.decommit_range(PAGE_SIZE, 4 * PAGE_SIZE)),
            Status::NOT_SUPPORTED.into_raw()
        );
        // decommit fails as expected
        assert_eq!(
            Status::result_into_raw(vmo.decommit_range(0, 4 * PAGE_SIZE)),
            Status::NOT_SUPPORTED.into_raw()
        );
        // decommit fails as expected
        assert_eq!(
            Status::result_into_raw(vmo.decommit_range(alloc_size - PAGE_SIZE, PAGE_SIZE)),
            Status::NOT_SUPPORTED.into_raw()
        );
    }

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

    /// Tests creating a zero-sized always-pinned VMO fails gracefully.
    #[test]
    fn vmo_always_pinned_with_no_pages_test() {
        // Verify that we don't trigger a panic during destruction of an always-pinned, but empty VMO.
        //
        // This is a regression test for https://fxbug.dev/511552403.

        let vmo = VmObjectPaged::create(ALLOC_FLAG_ANY, VmObjectPaged::ALWAYS_PINNED, 0);
        // Note that this call will fail.  That's because we've requested a zero-sized always-pinned
        // VMO, which is not a valid request.  However, under the hood, we'll make it far enough to create
        // the VMO even thought it will be destroyed before the call returns.
        assert_eq!(Status::result_into_raw(vmo.map(|_| ())), Status::INVALID_ARGS.into_raw());
    }
}
