// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

/// VMO tests duplicated from vmo_unittest.cc.
#[cfg(ktest)]
#[unittest::suite]
mod vmo_rs {
    use crate::vm::arch_vm_aspace::ARCH_MMU_FLAG_UNCACHED;
    use crate::vm::fault;
    use crate::vm::physical_page_borrowing_config::ScopedLoaningEnabled;
    use crate::vm::pinned_vm_object::PinnedVmObject;
    use crate::vm::pmm::{self, ALLOC_FLAG_ANY};
    use crate::vm::scanner::AutoVmScannerDisable;
    use crate::vm::vm_object::{EvictionHint, Resizability, SnapshotType, VmObject};
    use crate::vm::vm_object_paged::VmObjectPaged;
    use crate::vm::vm_object_physical::VmObjectPhysical;
    use crate::vm_unittests::test_helper::make_committed_pager_vmo;
    use page::SIZE as PAGE_SIZE_USIZE;
    use unittest::{
        assert_eq, assert_false, assert_ok, expect_eq, expect_false, expect_ok, expect_true,
        unwrap_ok,
    };
    use zx_status::Status;

    const PAGE_SIZE: u64 = PAGE_SIZE_USIZE as u64;

    /// Creates a vm object.
    #[test]
    fn vmo_create_test() {
        // Creates a vm object.
        let vmo = unwrap_ok!(VmObjectPaged::create(ALLOC_FLAG_ANY, 0, PAGE_SIZE));
        // vmo is not contig
        expect_false!(vmo.is_contiguous());
        // vmo is not resizable
        expect_false!(vmo.is_resizable());
    }

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

    /// Tests creating a physical VMO and checking its initial properties.
    #[test]
    fn vmo_create_physical_test() {
        // vm page allocation
        let (vm_page, pa) = unwrap_ok!(pmm::alloc_page(0));

        // vmobject creation
        let vmo = unwrap_ok!(VmObjectPhysical::create(pa, PAGE_SIZE_USIZE));
        let cache_policy = vmo.get_mapping_cache_policy();
        // check initial cache policy
        expect_eq!(ARCH_MMU_FLAG_UNCACHED, cache_policy);
        // check contiguous
        expect_true!(vmo.is_contiguous());

        drop(vmo);
        // SAFETY: vm_page was allocated via alloc_page above and is no longer referenced by vmo.
        unsafe { pmm::free_page(vm_page) };
    }

    /// Tests pinning ranges in a physical VMO.
    #[test]
    fn vmo_physical_pin_test() {
        let (vm_page, pa) = unwrap_ok!(pmm::alloc_page(0));

        let vmo = unwrap_ok!(VmObjectPhysical::create(pa, PAGE_SIZE_USIZE));

        // Validate we can pin the range.
        expect_ok!(vmo.commit_range_pinned(0, PAGE_SIZE, false));

        // Pinning out side should fail.
        expect_eq!(
            Status::result_into_raw(vmo.commit_range_pinned(PAGE_SIZE, PAGE_SIZE, false)),
            Status::OUT_OF_RANGE.into_raw()
        );

        // Unpin for physical VMOs does not currently do anything, but still call it to be API correct.
        vmo.unpin(0, PAGE_SIZE);

        drop(vmo);
        // SAFETY: vm_page was allocated via alloc_page above and is no longer referenced by vmo.
        unsafe { pmm::free_page(vm_page) };
    }

    /// Tests parent merging and user ID updates when VMO hierarchies collapse.
    #[test]
    fn vmo_parent_merge_test() {
        // Test that a VmObjectPaged that is only referenced by its children gets removed by effectively
        // merging into its parent and re-homing all the children. This should also drop any VmCowPages
        // being held open.
        let vmo = unwrap_ok!(VmObjectPaged::create(ALLOC_FLAG_ANY, 0, PAGE_SIZE));
        // Set a user ID for testing.
        vmo.set_user_id(42);

        let child = unwrap_ok!(vmo.create_clone(
            Resizability::NonResizable,
            SnapshotType::Full,
            0,
            PAGE_SIZE,
            false
        ));
        child.set_user_id(43);

        expect_eq!(vmo.parent_user_id(), 0);
        expect_eq!(vmo.user_id(), 42);
        expect_eq!(child.user_id(), 43);
        expect_eq!(child.parent_user_id(), 42);

        // Dropping the parent should re-home the child to an empty parent.
        drop(vmo);
        expect_eq!(child.user_id(), 43);
        expect_eq!(child.parent_user_id(), 0);

        drop(child);

        // Recreate a more interesting 3 level hierarchy with vmo->child->(child2,child3)

        let vmo = unwrap_ok!(VmObjectPaged::create(ALLOC_FLAG_ANY, 0, PAGE_SIZE));
        vmo.set_user_id(42);
        let child = unwrap_ok!(vmo.create_clone(
            Resizability::NonResizable,
            SnapshotType::Full,
            0,
            PAGE_SIZE,
            false
        ));
        child.set_user_id(43);
        let child2 = unwrap_ok!(child.create_clone(
            Resizability::NonResizable,
            SnapshotType::Full,
            0,
            PAGE_SIZE,
            false
        ));
        child2.set_user_id(44);
        let child3 = unwrap_ok!(child.create_clone(
            Resizability::NonResizable,
            SnapshotType::Full,
            0,
            PAGE_SIZE,
            false
        ));
        child3.set_user_id(45);

        expect_eq!(vmo.parent_user_id(), 0);
        expect_eq!(child.parent_user_id(), 42);
        expect_eq!(child2.parent_user_id(), 43);
        expect_eq!(child3.parent_user_id(), 43);

        // Drop the intermediate child, child2+3 should get re-homed to vmo
        drop(child);
        expect_eq!(child2.parent_user_id(), 42);
        expect_eq!(child3.parent_user_id(), 42);
    }

    /// Tests that writing to a VMO does not commit pages in its clone.
    #[test]
    fn vmo_write_does_not_commit_test() {
        let _scanner_disable = AutoVmScannerDisable::new();

        // Create a vmo and commit a page to it.
        let vmo = unwrap_ok!(VmObjectPaged::create(ALLOC_FLAG_ANY, 0, PAGE_SIZE));

        let val: u64 = 42;
        expect_ok!(vmo.write(0, &val.to_le_bytes()));

        // Create a CoW clone of the vmo.
        let clone = unwrap_ok!(vmo.create_clone(
            Resizability::NonResizable,
            SnapshotType::Full,
            0,
            PAGE_SIZE,
            false
        ));

        // Querying the page for read in the clone should return it.
        expect_ok!(clone.get_page_blocking(0, 0));

        // Querying for write, without any fault flags, should not work as the page is not committed in
        // the clone.
        expect_eq!(
            Status::result_into_raw(clone.get_page_blocking(0, fault::flag::WRITE)),
            Status::NOT_FOUND.into_raw()
        );

        // Adding a fault flag should cause the lookup to succeed.
        expect_ok!(clone.get_page_blocking(0, fault::flag::WRITE | fault::flag::SW_FAULT));
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

    /// Tests PinnedVmObject creation, move semantics, and RAII unpinning.
    #[test]
    fn vmo_pinned_wrapper_test() {
        // Porting note: This is a verbatim conversion of the C++ test `vmo_pinned_wrapper_test`.
        // It preserves move and assignment constructs from the C++ source that are trivial in Rust.

        {
            let vmo = unwrap_ok!(VmObjectPaged::create(ALLOC_FLAG_ANY, 0, PAGE_SIZE));
            let vmo = VmObjectPaged::into_vm_object(vmo);

            let mut pinned = unwrap_ok!(PinnedVmObject::create(vmo.clone(), 0, PAGE_SIZE, true));
            pinned = unwrap_ok!(PinnedVmObject::create(vmo, 0, PAGE_SIZE, true));
            drop(pinned);
        }

        {
            let vmo = unwrap_ok!(VmObjectPaged::create(ALLOC_FLAG_ANY, 0, PAGE_SIZE));
            let vmo = VmObjectPaged::into_vm_object(vmo);

            let mut pinned = Some(unwrap_ok!(PinnedVmObject::create(vmo, 0, PAGE_SIZE, true)));
            assert!(pinned.is_some());
            let empty: Option<PinnedVmObject> = None;
            pinned = empty;
            drop(pinned);
        }

        {
            let vmo = unwrap_ok!(VmObjectPaged::create(ALLOC_FLAG_ANY, 0, PAGE_SIZE));
            let vmo = VmObjectPaged::into_vm_object(vmo);

            let pinned = unwrap_ok!(PinnedVmObject::create(vmo, 0, PAGE_SIZE, true));
            let mut empty: Option<PinnedVmObject> = None;
            assert!(empty.is_none());
            empty = Some(pinned);
            drop(empty);
        }

        {
            let vmo = unwrap_ok!(VmObjectPaged::create(ALLOC_FLAG_ANY, 0, PAGE_SIZE));
            let vmo = VmObjectPaged::into_vm_object(vmo);

            let mut pinned1 = unwrap_ok!(PinnedVmObject::create(vmo.clone(), 0, PAGE_SIZE, true));
            let pinned2 = unwrap_ok!(PinnedVmObject::create(vmo, 0, PAGE_SIZE, true));
            pinned1 = pinned2;
            drop(pinned1);
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
