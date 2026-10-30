# Rust non test deps bazel alignment

- **Learned from:** inner loop friction on a local task (task `starnix-modules-usb-dev-devpts`)
- **Date:** 2026-10-24
- **Changed:** `checks/gn_attr_parity.sh`, `checks/manifest.json`

## Root cause

When a GN rustc_library or rustc_binary target using non_test_deps with
with_unit_tests = true is migrated to Bazel with a decomposed rustc_test target,
non_test_deps are merged directly into deps on the Bazel library/binary target.
While gn_dep_parity already recognized non_test_deps, gn_attr_parity did not
include non_test_deps in its dep_alignment or attribute_alignment INFO coverage,
leaving false-positive review_diff warnings for unmatched Bazel dependencies and
uncarried non_test_deps attributes.

## Why not a one-off fix

Any Fuchsia Rust crate that uses non_test_deps to swap production dependencies
for testutils in unit tests will merge non_test_deps into deps when migrating to
BUILD.bazel with an explicit rustc_test target, so gn_attr_parity must recognize
non_test_deps generically across all packages.
