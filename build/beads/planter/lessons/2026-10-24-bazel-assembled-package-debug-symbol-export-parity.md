# Bazel assembled package debug symbol export parity

> **Superseded (2026-10-26):** upstream policy is to never edit `build/bazel/debug_symbols/aspects.bzl`; keep the assembled `fuchsia_package` in GN instead (see `bazel_minimality` / `migration_sanity`).

- **Learned from:** cq failure on CL I1026055c72e63c9c569527e8ef9425b59043eb3d, patchset 1 (task `starnix-fxfs-crypt`)
- **Date:** 2026-10-24
- **Changed:** `checks/bazel_minimality.sh`, `checks/build_verification.sh`

## Root cause

When migrating an assembled Fuchsia package (fx_package + fx_packaged_binary)
that transitively depends on a rustc_dylib (rust_dylib_library) target,
fx_packaged_binary bundles the dylib .so into the package blobs and product
assembly elf_sizes.json, but build/bazel/debug_symbols/aspects.bzl only
collected rust_binary, rust_test, and rust_shared_library (omitting
rust_dylib_library). Local build_verification did not run
verify_elf_sizes_debug_symbols after fx build, and bazel_minimality blocked
edits to build/bazel/debug_symbols/aspects.bzl as out-of-scope.

## Why not a one-off fix

Fixing only a single package directory leaves any future fx_package migration
that transitively links a rustc_dylib or other uncollected binary kind
vulnerable to CQ export_last_build_debug_symbols failures and blocks the coder
from updating build/bazel/debug_symbols/aspects.bzl when required.
