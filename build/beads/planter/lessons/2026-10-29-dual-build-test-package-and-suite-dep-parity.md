# Dual build test package and suite dep parity

- **Learned from:** inner loop friction on a local task (task `starnix-module-procfs`)
- **Date:** 2026-10-29
- **Changed:** `checks/gn_dep_parity.sh`, `checks/manifest.json`

## Root cause

gn_dep_parity.sh only traversed GN group targets in reach() and treated any
target name present in BUILD.bazel (such as fx_test or fx_package) as still
existing in BUILD.gn (exists_now = True). When a GN fuchsia_unittest_package or
fuchsia_test_package was decomposed into fx_package(name = '<pkg>-package') +
fx_test(name = '<pkg>') (causing bazel2gn to generate
fuchsia_test_package('<pkg>-package') with package_name = '<pkg>-gn') and
exported via bazel_test_suite, or when a fuchsia_package was migrated to
fx_package, group('tests') replacing ':<pkg>' with ':<pkg>-package' and
':bazel_tests' was falsely flagged as dep_label_substituted.

## Why not a one-off fix

Hardcoding exemptions for a single package directory fails to cover all
dual-build migrations where GN test packages or non-test packages are decomposed
into fx_package + fx_test chains, wrapped in bazel_test_suite targets, or
migrated to Bazel-only fx_package rules across the tree.
