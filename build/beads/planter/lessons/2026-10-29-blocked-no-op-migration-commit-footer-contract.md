# Blocked/no-op migration commit footer contract (Bazel-Migration-Target and Test: footer requirements when a GN-only blocker leaves nothing migrated)

- **Learned from:** inner loop friction on CL I939014044406df42ab19f6d8e051bed4369e416e, patchset 2 (task `pkg-tests-system-update-committer`)
- **Date:** 2026-10-29
- **Changed:** `checks/commit_message_format.sh`, `checks/manifest.json`

## Root cause

commit_message_format only relaxed its footer rules for a completely empty
change (commit_subject_empty_change). A blocked migration that edits only
BUILD.gn (e.g. a comment naming the GN-only blocker after migration_sanity
forced the test back into GN) still had missing_bazel_migration_target_footer
demand a footer, while test_footer_missing_migrated_target demanded an fx bazel
build footer for a Bazel target that does not exist, so no commit message could
pass both.

## Why not a one-off fix

Any package whose migration is blocked by a GN-only dependency (RealmBuilder
servers, assembly-only packages, unmigrated deps) ends with a change that edits
no Bazel file. Waiving the finding for this one directory would leave every
future blocked task stuck in the same footer deadlock, so the check has to
recognize the nothing-migrated state in general.
