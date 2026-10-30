# Commit message accuracy for already migrated batch packages

- **Learned from:** human review on CL I9207755d31c8a45d069e3810da94d8ad5602adb5, patchset 1 (task `src-connectivity-bluetooth-examples-batch`)
- **Date:** 2026-10-28
- **Changed:** `checks/commit_message_format.sh`

## Root cause

When a batch migration task included package directories whose Bazel targets had
already landed in an earlier commit and only had their per-directory
bazel_test_suite BUILD.gn stubs consolidated into an ancestor test_suite
alongside newly migrated sibling packages, check_body_unmigrated_targets in
commit_message_format.sh only checked Migrate sentences against hand-written GN
targets kept above the sentinel and did not flag Migrate //<dir> prose claiming
to migrate a package directory where no targets were newly migrated against
base.

## Why not a one-off fix

Fixing the commit message only for the current batch of directories would not
prevent future batch migrations or post-rebase follow-up changes from leaving
stale Migrate //<dir> claims in the commit body for directories whose targets
were already migrated prior to the commit.
