# Bazel minimality ast non dep kwargs and restricted crate check

- **Learned from:** inner loop friction on a local task (task `starnix-module-ext4`)
- **Date:** 2026-10-27
- **Changed:** `checks/bazel_minimality.sh`, `checks/manifest.json`

## Root cause

In bazel_minimality.sh, check_restricted_rust_crate_deps and
check_missing_bazel_dependency_packages referenced NonDepKwargs when filtering
rule keyword arguments during Starlark AST traversal; ensuring NonDepKwargs is
defined with non-dependency keyword arguments and documenting
restricted_rust_crate_visibility in checks/manifest.json prevents runtime
NameError failures.

## Why not a one-off fix

Every migration package whose BUILD.bazel contains rule calls inspected for
missing dependency packages or restricted third-party Rust crate visibility
executes this AST keyword filtering path in bazel_minimality.sh, so the check
script definition must be maintained systemically across all targets.
