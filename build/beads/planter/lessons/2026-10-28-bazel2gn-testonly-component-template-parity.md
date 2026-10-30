# Bazel2gn testonly component template parity

- **Learned from:** inner loop friction on CL I4fda514490455dbe58280c229090f8d9f704d911, patchset 2 (task `src-security-tests-pkg-test-batch`)
- **Date:** 2026-10-28
- **Changed:** `checks/gn_target_type_parity.sh`, `checks/manifest.json`

## Root cause

In GN (//build/components/fuchsia_test_component.gni), a
`fuchsia_test_component` that does not set `test_type` is a thin wrapper around
`fuchsia_component` with `testonly = true`. When such a component (often a
testonly helper/mock component consumed by `deps` of a test package in another
directory) is migrated to `fx_component(testonly = True)` in `BUILD.bazel`,
`bazel2gn` generates `fuchsia_component(...) { testonly = true ... }` below `##
BAZEL2GN SENTINEL` because `bazel2gn` does not translate `fx_test_component`.
Both `gn_target_type_parity` and `review_diff` (`template_parity`) rejected the
`fuchsia_test_component` -> `fuchsia_component(testonly = true)` conversion even
when `test_type` was absent.

## Why not a one-off fix

Suppressing the finding only for specific test helper directories would fail to
handle any other dual-build package across the Fuchsia tree that migrates a
`fuchsia_test_component` without `test_type` to `fx_component(testonly = True)`
and syncs its `BUILD.gn` via `bazel2gn`.
