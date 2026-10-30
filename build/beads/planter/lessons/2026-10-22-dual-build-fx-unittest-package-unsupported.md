# Dual build fx unittest package unsupported attributes and parity coverage

- **Learned from:** cq failure on CL I1b7877d67dc8935cd5510603ec5304b01555ed81, patchset 3 (task `starnix-module-kgsl`)
- **Date:** 2026-10-22
- **Changed:** `checks/commit_message_format.sh`, `checks/gn_attr_parity.sh`, `checks/migration_sanity.sh`

## Root cause

In build/tools/bazel2gn/fx_unittest_package.go, fxUnittestPackageToGN only
supports the attributes name, package_name, max_log_severity, unit_tests,
visibility, tags, and testonly, requiring # @bazel2gn:skip on
fx_unittest_package and a hand-written fuchsia_unittest_package("<pkg>-gn")
above ## BAZEL2GN SENTINEL whenever any other attribute (such as test_type) is
set. However, checks/migration_sanity.sh only recognized resources = [...] as a
valid technical skip condition on fx_unittest_package, falsely flagging #
@bazel2gn:skip on fx_unittest_package(test_type = ...) as unjustified_test_skip
and the hand-written <pkg>-gn GN copy as duplicate_test_above_sentinel (which
previously pushed the coder into skipping rustc_test as well). Additionally,
checks/commit_message_format.sh flagged fx test //<pkg>:<name> when BUILD.gn
defined bazel_test_suite("<name>") even when BUILD.bazel defined a runnable
fx_unittest_package(name = "<name>") exported into tests.json, and
checks/gn_attr_parity.sh lacked INFO coverage for pre-existing sentinel BUILD.gn
files and bazel2gn path_overwrite/raw_attribute mappings.

## Why not a one-off fix

Fixing only a single package directory leaves checks/migration_sanity.sh
rejecting # @bazel2gn:skip and the paired <pkg>-gn GN test package for every
dual-build unit test package across the tree that sets test_type or other
fx_unittest_package attributes unsupported by bazel2gn, and leaves
checks/commit_message_format.sh and checks/gn_attr_parity.sh producing
false-positive errors on dual-build test suites and bazel2gn attribute mappings.
