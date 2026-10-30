# Dual build blocked test packaging and uncommitted worktree base detection

- **Learned from:** inner loop friction on a local task (task `starnix-modules-usb-dev-devpts`)
- **Date:** 2026-10-24
- **Changed:** `checks/cq_reachability.sh`, `checks/migration_sanity.sh`

## Root cause

First, bazel_packaging_blocked_reason in checks/migration_sanity.sh and
checks/cq_reachability.sh only matched literal fuchsia_unittest_package and
fuchsia_test_package blocks that directly contained ":<test_name>", missing GN
test packages above the sentinel that use fuchsia_test_with_expectations_package
(or other *_test*_package templates) or reference the generated test binary
indirectly via a local fuchsia_test_component target. Second,
check_unamended_working_tree_changes in checks/migration_sanity.sh treated HEAD
as an unsubmitted task commit whenever HEAD's commit message contained
Bazel-Migration-Target: even when PLANTER_CHANGE_BASE equaled HEAD and HEAD was
an already-landed upstream commit on origin/main.

## Why not a one-off fix

Any dual-build Rust package whose tests use
fuchsia_test_with_expectations_package or wrap the test binary in a separate
fuchsia_test_component above the sentinel will hit the same false-positive
rust_unit_test_layout / unpackaged_rust_device_test error, and any uncommitted
worktree session started when origin/main's tip commit is a landed migration CL
will trigger false-positive unamended_working_tree_changes errors across all
modified files unless both checks handle those general patterns.
