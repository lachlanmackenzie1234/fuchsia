# Stacked migration commit ownership detection

- **Learned from:** inner loop friction on a local task (task `starnix-modules-usb-dev-serial`)
- **Date:** 2026-10-24
- **Changed:** `checks/migration_sanity.sh`

## Root cause

When a task runs stacked on top of an unsubmitted parent migration commit with
PLANTER_CHANGE_BASE=HEAD and no uploaded commit yet,
head_is_unsubmitted_task_commit in migration_sanity.sh matched TAG: planter /
Bazel-Migration-Target: footers or loose substring/prefix matches against HEAD,
misidentifying the parent task commit as the current task's commit and reporting
false-positive unamended_working_tree_changes errors.

## Why not a one-off fix

Suppressing the finding for a single package directory would fail on every
stacked migration task whose parent commit carries planter footers, touches a
shared/child directory, or mentions the child package in its commit body.
