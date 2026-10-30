# Restricted third party rust crate visibility and multi dir diff reconciliation

- **Learned from:** inner loop friction on a local task (task `starnix-modules-fs`)
- **Date:** 2026-10-27
- **Changed:** `checks/bazel_minimality.sh`, `checks/gn_attr_parity.sh`, `checks/visibility_audit.sh`, `tools/run_checks.sh`

## Root cause

First, third-party Rust crates with restricted allowlists (such as once_cell,
regex, bincode, md5, and sha1) have package_group visibility on their
//third_party/rust_crates/vendor:<crate> aliases defined in
third_party/rust_crates/BUILD.bazel (paired with
third_party/rust_crates/visibility.gni in GN), but no static check verified that
migrated BUILD.bazel packages depending on a restricted crate were added to its
package_group in third_party/rust_crates/BUILD.bazel, nor did bazel_minimality
allow editing third_party/rust_crates/visibility.gni in scope. Second, when
multi-directory tasks ran visibility_audit, gn_attr_parity, and review_diff per
directory, PLANTER_TARGET_DIRS was overwritten with a single target_dir during
diffengine reconciliation and run_checks did not match existing INFO coverage
across line-number variations.

## Why not a one-off fix

Any Fuchsia Rust package migrating from GN to Bazel that depends on a
visibility-restricted third-party crate in //third_party/rust_crates/vendor will
fail Bazel analysis unless its package is registered in the corresponding
package_group in //third_party/rust_crates/BUILD.bazel (and cleaned up in
visibility.gni when GN targets no longer reference it), and any multi-package
migration batch needs consistent diffengine INFO reconciliation across all
target directories.
