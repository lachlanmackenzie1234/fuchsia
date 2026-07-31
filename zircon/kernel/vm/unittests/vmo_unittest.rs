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
    use crate::vm::vm_object_paged::VmObjectPaged;
    use page::SIZE as PAGE_SIZE_USIZE;
    use unittest::{assert_ok, unwrap_ok};

    const PAGE_SIZE: u64 = PAGE_SIZE_USIZE as u64;

    /// Tests HintRange(AlwaysNeed) evicts loaned pages.
    #[test]
    fn incomplete_vmo_always_need_evicts_loaned_test() {
        let _scanner_disable = AutoVmScannerDisable::new();

        // TODO(https://fxbug.dev/531878732): This test is intentionally incomplete. This test will
        // become progessively more complete as bindings for all of the constructs that the test
        // uses are added.

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
        }
    }
}
