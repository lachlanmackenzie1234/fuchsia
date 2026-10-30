# Test-helper component classification: GN components kept above the BAZEL2GN SENTINEL because their only consumers are testonly packages (subpackages of GN test packages) that bazel2gn cannot translate

- **Learned from:** inner loop friction on a local task (task `pkg-base-resolver-fake-deps`)
- **Date:** 2026-10-29
- **Changed:** `checks/migration_sanity.sh`

## Root cause

is_packaged_testonly_gn_binary walks a GN component's pre-sentinel consumers and
classified every fuchsia_package / fuchsia_package_with_single_component as a
non-test consumer, even when it is testonly and only exists as a subpackage of a
GN test package. A component consumed solely through such a testonly package
therefore lost the test-helper exemption and duplicate_target_above_sentinel
demanded migrating it, while its siblings referenced directly by
fuchsia_test_package were exempt, producing contradictory guidance.

## Why not a one-off fix

Any package whose GN test packages use testonly subpackages built from helper
components (a common pattern for resolver and subpackage integration tests) hits
the same misclassification; fixing only this directory (e.g. by renaming or
restructuring its BUILD.gn) would leave the check giving contradictory results
for every future migration with a testonly fuchsia_package between helper
components and GN test packages.
