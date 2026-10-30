# Bazel2gn generic rustc test output name collision

- **Learned from:** cq failure on CL I7691d2fed42bf601f99469e4d3e57d074e384643, patchset 4 (task `starnix-module-perfetto-consumer-perfetto_consumer`)
- **Date:** 2026-10-24
- **Changed:** `checks/gn_attr_parity.sh`, `checks/manifest.json`, `prompts/coder.md`

## Root cause

When decomposing a GN rustc_library("lib") or rustc_binary("bin") with custom
crate_name/output_name and with_unit_tests = true into an explicit Bazel
rustc_test or host_rustc_test with a generic target name (lib_test, bin_test,
test), bazel2gn emits rustc_test("lib_test") below the BAZEL2GN SENTINEL without
output_name, defaulting to $root_out_dir/lib_test (e.g. host_x64/lib_test)
instead of GN's <crate>_lib_test. Previously this rule only existed as prose in
prompts/coder.md with no deterministic static check in checks/gn_attr_parity.sh,
allowing migrations to land generic lib_test outputs that collide globally in GN
when two such packages coexist in the build graph.

## Why not a one-off fix

Any dual-build Rust library or binary package using the standard "lib" or "bin"
target naming pattern will hit this exact global $root_out_dir/lib_test
Duplicate output file collision in GN unless gn_attr_parity.sh deterministically
rejects unskipped generic rustc_test/host_rustc_test targets lacking a
disambiguated output_name across all packages.
