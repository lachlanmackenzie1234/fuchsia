# Dual build test skip parity

- **Learned from:** human review on CL I0d78d8d78b1d0101b8c0eb5845f015b0974217d1, patchset 1 (task `starnix-mpmc`)
- **Date:** 2026-10-22
- **Changed:** `checks/migration_sanity.sh`

## Root cause

When both a test binary rule (such as rustc_test) and its enclosing
fx_unittest_package were annotated with an unjustified '# @bazel2gn:skip' in a
dual-build package, has_technical_test_skip_condition() in
checks/migration_sanity.sh treated the fx_unittest_package's unit_tests
reference to the skipped test binary as satisfying the T1 (target-under-test
skipped) condition even when the underlying library under test was not skipped
and the test binary's own skip was unjustified. Additionally, the
unjustified_test_skip remediation did not warn about setting explicit crate_root
on standalone rustc_test targets with non-default entry points or avoiding
target-name collisions between bazel_test_suite and the bazel2gn-generated
fuchsia_unittest_package.

## Why not a one-off fix

Fixing only a single package's BUILD.bazel and BUILD.gn leaves
migration_sanity.sh masking unjustified skips on fx_unittest_package whenever
its inner unit_tests binary also carries an unjustified '# @bazel2gn:skip',
requiring an extra iteration round across any future dual-build library
migration with unit test packages.
