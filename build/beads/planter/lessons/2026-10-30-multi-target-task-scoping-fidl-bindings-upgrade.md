# Multi target task scoping fidl bindings upgrade and test type alignment

- **Learned from:** inner loop friction on CL I2ea94ff7f1265f1cb0439b8d2c9c8b0043802890, patchset 5 (task `starnix-modules-hw-power`)
- **Date:** 2026-10-30
- **Changed:** `checks/bazel_minimality.sh`, `checks/gn_attr_parity.sh`

## Root cause

First, bazel_minimality.sh enforced a fixed limit of 5 migrated directories and
550 added lines even when PLANTER_TARGET_DIRS explicitly assigned a larger batch
of directories to a single task, and rejected direct //sdk/fidl/<lib>
dependencies upgraded from legacy C++-only fuchsia_fidl_library to fidl_library
so Rust/C++ bindings exist in Bazel. Second, gn_attr_parity.sh did not record
INFO attribute_alignment coverage when a GN test package's test_type attribute
is carried on the decomposed fx_test_component target rather than the fx_test
target of the same name.

## Why not a one-off fix

Exempting only a specific FIDL package or module directory list would fail on
every future multi-package migration task that assigns more than five target
directories in PLANTER_TARGET_DIRS, depends on a //sdk/fidl/<lib> package that
still uses legacy fuchsia_fidl_library without Rust bindings, or unrolls a GN
unit test package with test_type into fx_test_component and fx_test.
