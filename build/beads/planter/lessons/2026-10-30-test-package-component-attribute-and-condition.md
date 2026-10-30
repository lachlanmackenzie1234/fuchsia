# Test package component attribute and condition parity

- **Learned from:** inner loop friction on CL Ifcab9bc2dbc8d4b619b21d6964f14d5a1763c285, patchset 2 (task `starnix-modules-usb-dev`)
- **Date:** 2026-10-30
- **Changed:** `checks/gn_attr_parity.sh`

## Root cause

When a GN fuchsia_unittest_package or fuchsia_test_package setting test_type
(alongside or without test_specs.log_settings.max_severity) is migrated to an
unrolled fx_test_component + fx_package + fx_test chain in BUILD.bazel,
test_type is placed on fx_test_component while review_diff pairs the original GN
test package by name with fx_test. Because
check_test_specs_log_severity_alignment_info in gn_attr_parity.sh only emitted
full-length permutations of extra_attrs (omitting single-attribute and subset
permutations such as 'test_type in GN not carried over to Bazel' and requiring
max_log_severity to be present), review_diff's warning on fx_test was not
deduplicated by an INFO finding from gn_attr_parity.

## Why not a one-off fix

Hardcoding a target-specific exemption for a single Starnix test package would
leave every other migrated fuchsia_unittest_package or fuchsia_test_package that
decomposes test_type onto a sibling fx_test_component vulnerable to the same
false-positive review_diff attribute_alignment warning.
