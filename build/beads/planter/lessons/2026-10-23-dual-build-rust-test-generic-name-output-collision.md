# Dual build rust test generic name output collision

- **Learned from:** cq failure on CL I6a8d8f31f50fd06b96a42f46aafc4bd9ee0c08cd, patchset 1 (task `starnix-fxfs-crypt-common`)
- **Date:** 2026-10-23
- **Changed:** `prompts/coder.md`

## Root cause

In GN, rustc_library("lib") with name/crate_name = "<crate>" and with_unit_tests
= true sets output_name = "<crate>_lib_test" on the generated lib_test target.
When decomposed into rustc_test(name = "lib_test", crate = ":lib") in Bazel and
translated by bazel2gn without # @bazel2gn:raw_attribute:output_name =
"<crate>_lib_test" (or renaming the test target to <crate>_lib_test), the
generated GN rustc_test("lib_test") defaults output_name to "lib_test" and
collides in $root_out_dir/lib_test with other packages' lib_test targets during
full-graph CQ builds.

## Why not a one-off fix

Many Rust libraries across the Fuchsia tree define rustc_library("lib") with a
custom crate_name and unit tests; fixing only a single package leaves all future
dual-build migrations of rustc_library("lib") / rustc_binary("bin") vulnerable
to the same global GN BUILDCONFIG.gn duplicate output file collision on CQ.
