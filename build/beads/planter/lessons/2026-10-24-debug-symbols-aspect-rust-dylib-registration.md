# Debug symbols aspect rust dylib registration

> **Superseded (2026-10-26):** upstream policy is to never edit `build/bazel/debug_symbols/aspects.bzl`; the `build/bazel/debug_symbols/` scope exemption and `missing_rust_dylib_debug_symbols_aspect` were removed.

- **Learned from:** inner loop friction on CL I1026055c72e63c9c569527e8ef9425b59043eb3d, patchset 1 (task `starnix-fxfs-crypt`)
- **Date:** 2026-10-24
- **Changed:** `checks/bazel_minimality.sh`, `checks/manifest.json`

## Root cause

is_shared_bazel_rule_file in bazel_minimality.sh treated every .bzl file under
build/bazel/ outside build/bazel/update-rustc-third-party/ as a forbidden shared
rule/macro file, rejecting legitimate edits to
build/bazel/debug_symbols/aspects.bzl that register rust_dylib_library in
_collect_debug_symbols_manifest_aspect_impl when a migrated fx_package packages
a rustc_dylib shared library.

## Why not a one-off fix

Any assembled fx_package migration whose component binary depends directly or
transitively on a rustc_dylib target (such as through shared component runtime
libraries) packages a Rust dylib .so ELF blob that CQ
export_last_build_debug_symbols requires in debug_symbols.json; allowing
build/bazel/debug_symbols/ updates in is_shared_bazel_rule_file and checking for
rust_dylib_library registration in aspects.bzl prevents both the false-positive
scope error and missing debug-symbol manifest failures across all future package
migrations.
