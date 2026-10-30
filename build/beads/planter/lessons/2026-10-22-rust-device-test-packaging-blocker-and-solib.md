# Rust device test packaging blocker and solib runfiles deduplication

> **Partly superseded (2026-10-26):** the transitive `depends_on_rustc_dylib` exemption was removed from `migration_sanity` to follow upstream; the `# Bazel test packaging blocked:` marker path remains.

- **Learned from:** cq failure on CL I13a4b9797326fcc4e9c660e87afe666cd1277850, patchset 4 (task `starnix-block-server`)
- **Date:** 2026-10-22
- **Changed:** `checks/manifest.json`, `checks/migration_sanity.sh`

## Root cause

In checkouts where build/bazel_sdk/fuchsia_rules_common/utils.bzl does not
deduplicate _solib_ shared-library symlinks in default_runfiles (such as after a
revert of _solib_ deduplication), packaging a rustc_test that transitively
depends on a rustc_dylib in fx_unittest_package fails Bazel analysis with
duplicate lib/lib*.so entries; furthermore, gn_test_package_not_migrated in
checks/migration_sanity.sh did not check bazel_packaging_blocked_reason on a
BUILD.bazel rustc_test paired with a hand-written GN test package above the
BAZEL2GN SENTINEL.

## Why not a one-off fix

Any dual-build Rust package whose unit test has a documented '# Bazel test
packaging blocked:' blocker in BUILD.bazel or transitively depends on a
rustc_dylib in a checkout without _solib_ runfiles deduplication will hit the
same contradiction between gn_test_package_not_migrated and
rust_test_packaging_blocked unless checks/migration_sanity.sh honors
bazel_packaging_blocked_reason alongside checkout_deduplicates_solib_runfiles
across all packages.
