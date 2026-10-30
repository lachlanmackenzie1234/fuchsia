# Bazel minimality ast dependency scanner scope

- **Learned from:** inner loop friction on a local task (task `starnix-modules-misc`)
- **Date:** 2026-10-27
- **Changed:** `checks/bazel_minimality.sh`

## Root cause

In checks/bazel_minimality.sh, the NonDepKwargs set used by
check_missing_bazel_dependency_packages and check_restricted_rust_crate_deps to
skip non-dependency rule keyword arguments was referenced inside the per-file
AST dependency inspection helpers before being defined at module scope, causing
a Python NameError when scanning BUILD.bazel target calls.

## Why not a one-off fix

Any migration task whose BUILD.bazel contains rule calls inspected by
check_missing_bazel_dependency_packages and check_restricted_rust_crate_deps
triggers this AST scan, so defining NonDepKwargs at module scope in
checks/bazel_minimality.sh is required across all package migrations rather than
fixing a single package.
