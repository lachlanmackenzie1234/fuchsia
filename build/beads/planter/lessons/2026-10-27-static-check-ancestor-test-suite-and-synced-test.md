# Static check ancestor test suite and synced test rdeps consistency

- **Learned from:** inner loop friction on CL Ib7799fbf9e2961e87bb3740435eed930c8b89743, patchset 8 (task `src-developer-ffx-lib-target`)
- **Date:** 2026-10-27
- **Changed:** `checks/cq_reachability.sh`, `tools/find_rdeps.sh`

## Root cause

checks/cq_reachability.sh contained an undefined call to
find_ancestor_bazel_test_suite(td, kind) and a duplicate 3-tuple unpacking of
anc_suite_info after nearest_ancestor_test_suite(td, param) had already resolved
the 2-tuple (anc_pkg, anc_suite), causing a NameError crash during test suite
reachability analysis; additionally, tools/find_rdeps.sh needed to match
checks/visibility_audit.sh in keeping bazel2gn-synced test targets below
BAZEL2GN SENTINEL eligible for explicit GN caller rdeps while excluding them
from default-target and forwarding-group resolution.

## Why not a one-off fix

A check script crash or mismatch between find_rdeps and visibility_audit affects
every dual-build migration that exports Bazel tests through an ancestor
test_suite or syncs test targets below BAZEL2GN SENTINEL, so fixing the check
and tool scripts directly is required across all packages.
