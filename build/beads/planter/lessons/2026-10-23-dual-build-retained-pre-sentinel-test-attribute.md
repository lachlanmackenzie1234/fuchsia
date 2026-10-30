# Dual build retained pre sentinel test attribute alignment

- **Learned from:** inner loop friction on CL I0f132cfe7d44f413c1b7e1fa9f7632455d04d455, patchset 1 (task `starnix-modules-usb-dev-nanohub`)
- **Date:** 2026-10-23
- **Changed:** `checks/gn_attr_parity.sh`

## Root cause

When a dual-build Rust library decomposes with_unit_tests into a hand-written
rustc_test('<name>_test') and fuchsia_unittest_package retained above the
BAZEL2GN SENTINEL in BUILD.gn (because exclude_toolchain_tags or a test runtime
dependency is GN-only), gn_attr_parity.sh failed to emit matching INFO
attribute_alignment coverage for (1) rustc_library targets combining added
crate_name in Bazel with a pre-sentinel decomposed unit test and test-only
exclude_toolchain_tags preserved on <name>_test, and (2) retained pre-sentinel
GN test package targets that diffengine pairs with unrelated Bazel-only
fx_package targets in BUILD.bazel.

## Why not a one-off fix

Any dual-build package whose unit tests must stay hand-written above the
BAZEL2GN SENTINEL (such as tests with GN-only exclude_toolchain_tags or GN-only
runtime dependencies) or whose BUILD.bazel also defines Bazel-only fx_package
targets will trigger the same false-positive review_diff attribute_alignment
warnings unless gn_attr_parity.sh systematically records INFO coverage for
pre-sentinel decomposed unit test attributes and retained pre-sentinel target
pairings across all packages.
