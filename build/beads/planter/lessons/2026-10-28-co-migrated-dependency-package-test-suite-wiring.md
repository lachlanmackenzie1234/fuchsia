# Co-migrated dependency package test suite wiring and decomposed unit test lint config preservation

- **Learned from:** human review on CL If360d6314b1adc3c4597bbbdcaea551770e5b024, patchset 6 (task `src-sys-run-test-suite`)
- **Date:** 2026-10-28
- **Changed:** `checks/build_only_scope.sh`, `checks/cq_reachability.sh`, `checks/gn_attr_parity.sh`, `checks/lint_config_parity.sh`

## Root cause

First, cq_reachability.sh iterated only over PLANTER_TARGET_DIRS instead of all
newly migrated or dual-build packages modified in the change, so co-migrated
dependency packages escaped the check against leaf per-package bazel_test_suite
definitions instead of wiring package test_suite targets into ancestor
aggregation suites. Second, gn_attr_parity.sh did not verify that explicit
rustc_test/host_rustc_test targets decomposed from with_unit_tests preserve the
crate's lint_config, while build_only_scope.sh and lint_config_parity.sh blocked
minimal test-code lint fixes required to retain that lint_config when the unit
test was not built in the default GN graph.

## Why not a one-off fix

Fixing only a single package directory would leave any future multi-package
migration able to add leaf bazel_test_suite blocks in co-migrated dependency
packages or silently drop lint_config from decomposed unit test targets across
the tree.
