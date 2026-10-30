# Preexisting debug symbols aspect scope isolation

> **Partly superseded (2026-10-26):** the `build/bazel/debug_symbols/` scope exemption in `bazel_minimality` was removed; upstream policy is to never edit `aspects.bzl`.

- **Learned from:** inner loop friction on CL I733d05125173d968298e788a9a4ec995ea38f014, patchset 14 (task `starnix-core`)
- **Date:** 2026-10-24
- **Changed:** `checks/bazel_minimality.sh`, `checks/build_verification.sh`, `checks/manifest.json`

## Root cause

When a stacked parent commit migrates an assembled fx_package that bundles a
rustc_dylib without updating build/bazel/debug_symbols/aspects.bzl, downstream
library migrations on top of that stack encounter an
export_last_build_debug_symbols failure on the parent package's blob. In
build_verification.sh, is_in_scope_debug_symbol_failure unconditionally returned
True whenever any changed package defined a shared library rule without matching
the library name against the failing blob install path, and did not exclude
third_party/rust_crates visibility edits from changed package directories;
meanwhile bazel_minimality.sh unconditionally exempted
build/bazel/debug_symbols/ from shared-rule scope checks even when no in-scope
package defined or packaged a rustc_dylib, creating a conflict with build-only
scope review when the coder followed the remediation.

## Why not a one-off fix

Exempting a single package or target directory would leave any other stacked
migration task vulnerable to false-positive export_debug_symbols errors from
parent commits and contradictory scope vs. aspect remediation rules whenever an
assembled package in an ancestor commit is missing debug symbol collection.
