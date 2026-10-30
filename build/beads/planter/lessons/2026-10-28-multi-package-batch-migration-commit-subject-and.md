# Multi-package batch migration commit subject and bazel test suite cleanup classification

- **Learned from:** inner loop friction on CL I9207755d31c8a45d069e3810da94d8ad5602adb5, patchset 1 (task `src-connectivity-bluetooth-examples-batch`)
- **Date:** 2026-10-28
- **Changed:** `checks/commit_message_format.sh`

## Root cause

In checks/commit_message_format.sh, is_dir_migrated_in_commit treated any
deleted BUILD.gn as a newly migrated package even when the base BUILD.gn only
contained bazel_test_suite and group("tests") for a package already migrated in
an earlier commit, and check_subject_partial_directory rejected a commit subject
naming a common parent directory that summarizes multiple migrated or modified
child packages under that parent as commit_subject_already_migrated_directory.

## Why not a one-off fix

Any multi-package batch migration or test-suite consolidation across sibling
subdirectories under a shared parent directory can delete bazel_test_suite-only
BUILD.gn files in already-migrated siblings while naming the shared parent
directory in the [bazel_migration] //<parent_dir> commit subject, so
commit_message_format.sh must distinguish meta-only BUILD.gn deletions from new
package migrations and accept common parent directory subjects for multi-package
changes.
