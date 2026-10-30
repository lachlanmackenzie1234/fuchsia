# Stale missing bazel dependency and resource packaged test migration

- **Learned from:** human review on CL If876caff81e2bcda299c8a695eb6eed71aa22cfa, patchset 3 (task `starnix-ext4-lib`)
- **Date:** 2026-10-23
- **Changed:** `checks/manifest.json`, `checks/migration_sanity.sh`

## Root cause

checks/migration_sanity.sh treated local resource() and resource_group() labels
in fuchsia_unittest_package/fuchsia_test_package deps as unknown-language test
binaries (returning None from test_language), which caused
gn_test_package_not_migrated to skip any GN test package bundling resources
alongside a migratable test binary, and did not flag comments claiming a
dependency has no Bazel build when that dependency's BUILD.bazel now defines the
target.

## Why not a one-off fix

Fixing only a single package leaves migration_sanity blind to other GN test
packages that bundle resource() or resource_group() targets and to stale
missing-Bazel-dependency blocker comments after upstream dependencies land in
BUILD.bazel.
