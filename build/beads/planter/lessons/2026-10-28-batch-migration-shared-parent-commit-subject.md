# Batch migration shared parent commit subject validation

- **Learned from:** inner loop friction on CL I9207755d31c8a45d069e3810da94d8ad5602adb5, patchset 4 (task `src-connectivity-bluetooth-examples-batch`)
- **Date:** 2026-10-28
- **Changed:** `checks/commit_message_format.sh`, `tools/run_checks.sh`

## Root cause

When a multi-directory batch migration commit uses the shared parent directory
in its `[bazel_migration][<subsystem>] //<parent_dir>` subject line,
`commit_message_format.sh` rejected the parent subject under single-target
invocations (`len(dirs) == 1`, including `commit_msg_only` runs in
`run_checks.sh` that truncated `target_dirs` to `target_dirs[:1]` before
populating `PLANTER_TARGET_DIRS`) and when sibling subdirectories in the batch
completed Case 1 migration by deleting `bazel_test_suite`-only `BUILD.gn` files.

## Why not a one-off fix

Hardcoding an exception for a single subsystem or directory would fail on all
future multi-package batch migrations across the Fuchsia tree where reviewers
request a common parent directory commit subject or where `run_checks.sh`
re-verifies commit-message-only amendments after upload.
