# Post rebase upstream rdep visibility audit

- **Learned from:** cq failure on CL I13a4b9797326fcc4e9c660e87afe666cd1277850, patchset 7 (task `starnix-block-server`)
- **Date:** 2026-10-23
- **Changed:** `checks/visibility_audit.sh`, `tools/run_checks.sh`

## Root cause

When an upstream commit on origin/main adds a new BUILD.gn or BUILD.bazel
reverse dependency on a migrated target after the initial migration commit was
authored, subsequent review rounds that rebase onto origin/main and only amend
the commit message hit two blind spots: (1) tools/run_checks.sh excluded
visibility_audit from COMMIT_MSG_CHECKS and truncated PLANTER_TARGET_DIRS to
only the first directory when commit_msg_only_since_upload() held after a
rebase, and (2) checks/visibility_audit.sh did not normalize a pre-rebase
PLANTER_CHANGE_BASE (where base is no longer an ancestor of HEAD) or a clean
HEAD default to HEAD~1 and did not collect all directories from
PLANTER_TARGET_DIRS.

## Why not a one-off fix

Updating visibility on a single target directory only fixes the current CL; any
future multi-directory or rebased migration where origin/main introduces a new
caller before a rebase or commit-message-only round will fail on CQ with
'Dependency not allowed' unless run_checks.sh always runs visibility_audit
across all PLANTER_TARGET_DIRS and visibility_audit.sh normalizes post-rebase
change bases to HEAD~1.
