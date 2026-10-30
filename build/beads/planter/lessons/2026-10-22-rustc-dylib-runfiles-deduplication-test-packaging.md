# Rustc dylib runfiles deduplication test packaging blocker

> **Partly superseded (2026-10-26):** the transitive `depends_on_rustc_dylib` exemption was removed from `migration_sanity` to follow upstream; the `_solib_` blocker-marker check remains.

- **Learned from:** inner loop friction on CL I13a4b9797326fcc4e9c660e87afe666cd1277850, patchset 4 (task `starnix-block-server`)
- **Date:** 2026-10-22
- **Changed:** `checks/cq_reachability.sh`, `checks/migration_sanity.sh`

## Root cause

The static checks migration_sanity.sh and cq_reachability.sh unconditionally
assumed that //build/bazel_sdk/fuchsia_rules_common/utils.bzl deduplicated
_solib_ shared-library symlinks in default_runfiles, omitting the transitive
rustc_dylib dependency check on GN test packages and rejecting
_solib_/rustc_dylib blocker markers even in checkouts where _solib_ runfiles
deduplication is absent or reverted.

## Why not a one-off fix

Any Fuchsia Rust unit test package whose test binary directly or transitively
depends on a rustc_dylib or rust_dylib_library target fails fx_packaged_binary
and fx_unittest_package analysis with duplicate lib/*.so entries whenever
utils.bzl does not deduplicate _solib_ runfiles symlinks, so the checks must
dynamically inspect utils.bzl and transitive test dependencies across all
packages.
