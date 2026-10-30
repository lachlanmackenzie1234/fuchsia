# Bazel test packaging resolved rustc dylib runfiles blocker

- **Learned from:** human review on CL I13a4b9797326fcc4e9c660e87afe666cd1277850, patchset 2 (task `starnix-block-server`)
- **Date:** 2026-10-22
- **Changed:** `checks/cq_reachability.sh`, `checks/migration_sanity.sh`

## Root cause

checks/migration_sanity.sh still contained the depends_on_rustc_dylib exemption
for gn_test_package_not_migrated, and bazel_packaging_blocked_reason in both
checks/migration_sanity.sh and checks/cq_reachability.sh accepted '# Bazel test
packaging blocked:' comments citing the resolved duplicate _solib_ / rustc_dylib
shared-library runfiles bug instead of requiring fx_unittest_package migration.

## Why not a one-off fix

Fixing only a single package directory leaves the stale depends_on_rustc_dylib
exemption in checks/migration_sanity.sh and allows '# Bazel test packaging
blocked:' comments citing duplicate _solib_ / rustc_dylib shared-library
runfiles to bypass rust_unit_test_layout and unpackaged_rust_device_test across
all future Rust test migrations.
