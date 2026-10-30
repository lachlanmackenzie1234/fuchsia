# Test target and host test parity

- **Learned from:** cq failure on CL I9665eb5ba5b0b2bbfbde82ebcc1403a9202fd833, patchset 18 (task `starnix-core-net-leaves-fidl_fuchsia_net_neighbor_ext`)
- **Date:** 2026-10-22
- **Changed:** `checks/cq_reachability.sh`, `checks/gn_attr_parity.sh`, `checks/manifest.json`

## Root cause

Existing checks (`cq_reachability`, `migration_sanity`, `gn_target_type_parity`)
only audited test targets present in the migrated `BUILD.bazel`/`BUILD.gn`
rather than verifying that all `fuchsia_unittest_package`/`fuchsia_test_package`
targets and `:<test>($host_toolchain)` `group("tests")` entries from the base
`BUILD.gn` at `$PLANTER_CHANGE_BASE` were preserved, and `gn_attr_parity` had a
helper (`check_unreferenced_variant_test_pkg_info`) that suppressed dropped
variant unit-test packages as non-blocking INFO.

## Why not a one-off fix

Fixing only the single package leaves future migrations of multi-variant or dual
target/host Rust and C++ libraries vulnerable to silently dropping secondary
`fuchsia_unittest_package` targets or `($host_toolchain)` test entries from
`BUILD.bazel`, `bazel_test_suite`, and `group("tests")` without any
deterministic check failure.
