# Multi-directory parallel migration scope isolation and restricted third-party Rust crate allowlist updates

- **Learned from:** inner loop friction on a local task (task `starnix-modules-fs`)
- **Date:** 2026-10-29
- **Changed:** `checks/bazel_minimality.sh`, `tools/run_checks.sh`

## Root cause

When a coder invoked run_checks.sh with a single directory argument in a
multi-directory migration task, run_checks.sh overwrote PLANTER_TARGET_DIRS in
the subprocess environment with only the CLI directory argument instead of
preserving the full PLANTER_TARGET_DIRS list, causing bazel_minimality to report
false-positive out_of_scope_package_modified errors on sibling target
directories and surfacing findings from unselected sibling packages.
Additionally, the restricted_rust_crate_visibility remediation did not
explicitly state that third_party/rust_crates/BUILD.bazel is an allowed in-scope
file that should be edited directly in the change.

## Why not a one-off fix

Fixing only a single package directory or manually patching one crate visibility
group would not prevent run_checks.sh from clobbering PLANTER_TARGET_DIRS and
reporting sibling package edits as out-of-scope errors across future
multi-directory migration tasks.
