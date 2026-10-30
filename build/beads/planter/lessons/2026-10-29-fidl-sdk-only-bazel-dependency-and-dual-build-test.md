# Fidl sdk only bazel dependency and dual build test package reachability

- **Learned from:** inner loop friction on a local task (task `starnix-modules-hw-power`)
- **Date:** 2026-10-29
- **Changed:** `checks/bazel_minimality.sh`, `checks/build_only_scope.sh`

## Root cause

First, during parallel sub-coder execution, gn_dep_parity flagged group("tests")
replacing a GN fuchsia_unittest_package with its bazel2gn-generated
<pkg>-package and bazel_test_suite (since resolved by gn_test_package_reached in
gn_dep_parity.sh). Second, bazel_minimality's
check_missing_bazel_dependency_packages only verified that a referenced
<dep_pkg>/BUILD.bazel file existed on disk, missing FIDL packages whose
BUILD.bazel only declared @rules_fuchsia fuchsia_fidl_library (which only
generates SDK C++ bindings) without an in-tree fidl_library target when a
consumer depended on :<lib>_rust, :<lib>_rust_next, :<lib>_hlcpp, or
:<lib>_banjo. In addition, build_only_scope and bazel_minimality did not allow
updating scripts/shac/bazel_migration_fidl.star or
scripts/shac/bazel_lint/allowlists/*.json when migrating such a FIDL package.

## Why not a one-off fix

Any first-party Rust or HLCPP target that depends on a FIDL library whose
pre-existing BUILD.bazel only defines @rules_fuchsia fuchsia_fidl_library will
pass static file-existence checks while failing Bazel analysis in fx build and
fx bazel build unless check_missing_bazel_dependency_packages inspects the
dependency BUILD.bazel for fidl_library and allows the corresponding shac
allowlist updates.
