# Bazel2gn raw attribute alignment parity

- **Learned from:** cq failure on CL I892a9ed8a7c2db20713c5c51833aa8d855ae16d0, patchset 6 (task `starnix-kernel-manager`)
- **Date:** 2026-10-22
- **Changed:** `checks/gn_attr_parity.sh`, `checks/manifest.json`

## Root cause

When a migrated Rust target in BUILD.bazel maps its lint config to lint_config,
decomposes with_unit_tests = true into an explicit rustc_test target so
dual-testing runs in both GN and Bazel, and preserves target-local GN
variables/configs (such as _concurrent_jobs = concurrent_jobs.<pool> and configs
+= _concurrent_jobs.configs) in the generated BUILD.gn via #
@bazel2gn:raw_attribute, review_diff compares the base BUILD.gn AST against
BUILD.bazel keyword arguments and emits a false-positive blocking
attribute_alignment WARNING because gn_attr_parity.sh only parsed quoted strings
in configs and only emitted attribute_alignment INFO coverage for
rustc_library_with_features or file-local templates.

## Why not a one-off fix

Fixing only a single package directory is insufficient because many Fuchsia Rust
and C++ targets across multiple subsystems use target-local GN variable
assignments and config appends (such as _concurrent_jobs =
concurrent_jobs.link_highmem or concurrent_jobs.rust_highmem with configs +=
_concurrent_jobs.configs) preserved via # @bazel2gn:raw_attribute alongside
lint_config and decomposed unit tests.
