# Rust test clippy warn all production lint parity

- **Learned from:** human review on CL If360d6314b1adc3c4597bbbdcaea551770e5b024, patchset 8 (task `src-sys-run-test-suite`)
- **Date:** 2026-10-28
- **Changed:** `checks/lint_config_parity.sh`

## Root cause

In Bazel, //build/config/rust/lints:clippy_warn_all merges
_CLIPPY_WARN_PRODUCTION (which enables nursery lints such as
clippy::redundant_clone), whereas in GN rustc_test omits clippy_warn_production
and clippy_warn_all only adds -Wclippy::all. When decomposing a GN
rustc_library(with_unit_tests = true) with clippy_warn_all into an explicit
rustc_test, setting lint_config = "//build/config/rust/lints:clippy_warn_all" on
the test target triggers production-only clippy lints on #[cfg(test)] code,
causing the coder to make prohibited lint-driven source edits.

## Why not a one-off fix

Reverting the .clone() edit in a single crate without explaining the Bazel vs GN
clippy_warn_all divergence on rustc_test targets in lint_config_parity
remediation leaves future migrations of crates using clippy_warn_all with unit
tests vulnerable to the same lint trap and prohibited source modifications.
