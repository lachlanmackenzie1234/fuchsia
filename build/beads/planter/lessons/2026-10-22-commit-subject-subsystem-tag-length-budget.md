# Commit subject subsystem tag length budget conflict

- **Learned from:** inner loop friction on CL I3636f35bb4f35a4305e047281d4202be39a8745d, patchset 12 (task `starnix-net-filter-ext`)
- **Date:** 2026-10-22
- **Changed:** `checks/commit_message_format.sh`, `checks/confidentiality_check.sh`, `checks/manifest.json`

## Root cause

checks/commit_message_format.sh unconditionally emitted
commit_subject_missing_tag whenever a commit subject used '[bazel_migration]
//<dir>' and git history had recurring subsystem tags, without checking whether
prepending any '[<subsystem>]' tag to the full directory path would exceed the
65-character subject limit enforced by checks/confidentiality_check.sh
(commit_msg_subject_length).

## Why not a one-off fix

Any deeply nested Fuchsia package directory whose full path brings
'[bazel_migration] //<dir>' close to 65 characters will trigger an unresolvable
loop between commit_subject_missing_tag and commit_msg_subject_length unless
commit_message_format.sh checks the 65-character budget before requiring a
subsystem tag on an unabbreviated path.
