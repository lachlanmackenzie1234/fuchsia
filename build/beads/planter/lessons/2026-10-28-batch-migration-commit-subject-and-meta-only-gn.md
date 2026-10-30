# Batch migration commit subject and meta only gn deletion classification

- **Learned from:** human review on CL I9207755d31c8a45d069e3810da94d8ad5602adb5, patchset 4 (task `src-connectivity-bluetooth-examples-batch`)
- **Date:** 2026-10-28
- **Changed:** `checks/commit_message_format.sh`

## Root cause

In commit_message_format.sh, is_dir_migrated_in_commit() treated any deleted
BUILD.gn file as a new target migration even when all targets in BUILD.bazel
already existed at base and the deleted BUILD.gn only contained
bazel_test_suite/group meta constructs. In addition,
check_subject_partial_directory() rejected commit subjects naming a shared
parent directory across a multi-directory batch migration with
commit_subject_already_migrated_directory, forcing the coding agent to revert
the commit subject back to an already-migrated leaf directory.

## Why not a one-off fix

Batch migrations across sibling subdirectories routinely combine new package
migrations with test_suite consolidation of already-migrated sibling packages
and use the shared parent directory in the commit subject; fixing only a single
directory path would leave commit_message_format.sh rejecting parent-directory
batch subjects and misclassifying meta-only BUILD.gn deletions across the tree.
