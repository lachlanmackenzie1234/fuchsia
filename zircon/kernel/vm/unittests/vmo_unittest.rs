// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

/// VMO tests duplicated from vmo_unittest.cc.
#[cfg(ktest)]
#[unittest::suite]
mod vmo_rs {
    /// Tests HintRange(AlwaysNeed) evicts loaned pages.
    #[test]
    fn incomplete_vmo_always_need_evicts_loaned_test() {
        // TODO(https://fxbug.dev/531878732): This test is intentionally incomplete. This test will
        // become progessively more complete as bindings for all of the constructs that the test
        // uses are added.

        let try_count = 30;
        for _try_ordinal in 0..try_count {}
    }
}
