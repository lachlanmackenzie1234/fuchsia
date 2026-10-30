# Dual build local test suite and parallel batch test wiring

- **Learned from:** inner loop friction on a local task (task `src-performance-memory-batch`)
- **Date:** 2026-10-28
- **Changed:** `checks/build_verification.sh`, `checks/cq_reachability.sh`, `checks/gn_dep_parity.sh`, `checks/gn_target_type_parity.sh`, `checks/visibility_audit.sh`

## Root cause

Three machinery gaps collided during parallel batch dual-build migrations: (1)
cq_reachability's preferred_ancestor_area_suite required creating a brand-new
top-level //<area>/BUILD.bazel test_suite even for dual-build packages that
retain BUILD.gn with a BAZEL2GN SENTINEL and for multi-package parallel batches
where creating a new ancestor BUILD.bazel is outside leaf coder scope, while
visibility_audit flagged ancestor package visibility on test targets before the
ancestor BUILD.bazel existed; (2) gn_target_type_parity and gn_dep_parity only
permitted replacing a GN fuchsia_unittest_package("tests") with group("tests")
when the leaf BUILD.gn itself declared a local bazel_test_suite, failing
whenever tests were exported through an ancestor test_suite or wrapped a
bazel2gn-generated fuchsia_unittest_package; and (3) build_verification did not
recognize pre-existing Go toolchain/third_party/golibs version mismatches in
host tool actions outside non-Go migrated packages.

## Why not a one-off fix

Fixing only the individual target directories would leave every future
dual-build package whose pre-migration BUILD.gn named its
fuchsia_unittest_package "tests", every parallel batch migration in an area
without an existing top-level BUILD.bazel test_suite, and every non-Go migration
running in a checkout with a pre-existing Go SDK/golibs mismatch blocked on
contradictory static and build verification errors.
