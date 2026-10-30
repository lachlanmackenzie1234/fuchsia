# Direct legacy fidl sdk dependency upgrade scoping and unloadable subpackage no sandbox exemption

- **Learned from:** inner loop friction on CL I2ea94ff7f1265f1cb0439b8d2c9c8b0043802890, patchset 4 (task `starnix-modules-hw-power`)
- **Date:** 2026-10-30
- **Changed:** `checks/bazel_minimality.sh`

## Root cause

In checks/bazel_minimality.sh, migrated_dependency_packages() only allowed
co-migrated dependency directories outside target_dirs when they were in the
same subsystem area and eliminated BUILD.gn or BAZEL2GN SENTINEL, rejecting
direct FIDL dependency packages whose pre-existing BUILD.bazel only defined a
legacy @rules_fuchsia fuchsia_fidl_library target (which emits no in-tree _rust
or platform bindings) and had to be upgraded to fidl_library and synced with fx
bazel2gn for an assigned target directory to compile. Additionally,
check_no_sandbox_tags flagged documented no-sandbox tags when an included header
lived under an upstream submodule BUILD.bazel containing an empty glob() pattern
that fails Bazel package loading under --incompatible_disallow_empty_glob.

## Why not a one-off fix

Hardcoding an exception for a single FIDL package or library directory would
fail whenever any other assigned target directory depends on a FIDL library
whose pre-existing BUILD.bazel still uses legacy @rules_fuchsia
fuchsia_fidl_library without in-tree fidl_library bindings, or includes headers
from an upstream submodule whose BUILD.bazel has an empty glob blocking
subpackage loading.
