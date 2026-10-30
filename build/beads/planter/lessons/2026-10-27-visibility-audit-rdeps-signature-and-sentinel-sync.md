# Visibility audit rdeps signature and sentinel sync

- **Learned from:** inner loop friction on CL I110b0cec1edf55fd56c7800f10d3e4fbb587b407, patchset 1 (task `src_sys_time_httpsdate_time_source_tests`)
- **Date:** 2026-10-27
- **Changed:** `checks/visibility_audit.sh`

## Root cause

A prior evolution of checks/visibility_audit.sh updated the call site of
compute_package_rdeps() to pass 7 positional arguments (including
test_rule_targets) and added a 3-tuple unpack of
compute_gn_nonconvertible_targets() with below_sentinel, while leaving def
compute_package_rdeps() with only 6 parameters and def
compute_gn_nonconvertible_targets() referencing an undefined below_synced
variable without a below_sentinel parameter, causing visibility_audit.sh to
crash with TypeError on every run.

## Why not a one-off fix

Because visibility_audit.sh runs on every GN-to-Bazel migration task, a mismatch
in compute_package_rdeps() and compute_gn_nonconvertible_targets() crashes the
check across all packages; fixing the function signatures, below-sentinel target
extraction, and test_rule_targets filtering in checks/visibility_audit.sh
resolves the crash systemically across all migrations.
