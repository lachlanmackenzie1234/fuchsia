# Static check ast scope and restricted rust crate visibility

- **Learned from:** inner loop friction on a local task (task `starnix-modules-fs`)
- **Date:** 2026-10-27
- **Changed:** `checks/bazel_minimality.sh`

## Root cause

In checks/bazel_minimality.sh, NonDepKwargs was defined locally inside
check_missing_bazel_dependency_packages instead of the enclosing per-file scope,
causing check_restricted_rust_crate_deps to fail with NameError: name
'NonDepKwargs' is not defined whenever scanning rule call keyword arguments.

## Why not a one-off fix

Because check_restricted_rust_crate_deps runs on every migrated BUILD.bazel file
when third_party/rust_crates defines restricted crate package_groups, leaving
NonDepKwargs scoped inside check_missing_bazel_dependency_packages crashes
bazel_minimality across all migrations rather than a single package.
