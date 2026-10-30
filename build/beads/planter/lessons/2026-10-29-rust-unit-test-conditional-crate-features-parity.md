# Rust unit test conditional crate features parity

- **Learned from:** inner loop friction on a local task (task `starnix-module-kgsl`)
- **Date:** 2026-10-29
- **Changed:** `checks/gn_attr_parity.sh`, `checks/manifest.json`, `tools/run_checks.sh`

## Root cause

When a GN Rust target with features (including features conditionally appended
in if-blocks) and with_unit_tests = true is migrated to an explicit Bazel
rustc_test target that gates crate_features using Starlark conditional
expressions (e.g., (["<feat>"] if <arg> else []) + ...) or top-level variables,
review_diff's unit_test_parity check extracts all features from GN if-blocks on
the library but fails to descend into Starlark IfExp/BinOp nodes on
rustc_test.crate_features, falsely reporting that the unit test omits those
features.

## Why not a one-off fix

Any Fuchsia Rust library or binary that gates Cargo features on GN
declare_args() build arguments exported via @fuchsia_build_info//:args.bzl must
use Starlark conditional expressions for crate_features on both the library and
its decomposed rustc_test target so bazel2gn regenerates matching if-blocks in
BUILD.gn; suppressing or hardcoding a single package would leave all other
feature-gated Rust unit test migrations blocked on the same false-positive
unit_test_parity error.
