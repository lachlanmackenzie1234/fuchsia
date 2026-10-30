# Commit message test suite and explicit target name resolution

- **Learned from:** inner loop friction on CL I733d05125173d968298e788a9a4ec995ea38f014, patchset 18 (task `starnix-core`)
- **Date:** 2026-10-26
- **Changed:** `checks/commit_message_format.sh`

## Root cause

checks/commit_message_format.sh had two false-positive heuristics: (1)
test_footer_not_ancestor_suite scanned ancestor test_suite() targets for every
directory in PLANTER_TARGET_DIRS without checking whether that directory
actually migrated new host test targets versus $PLANTER_CHANGE_BASE or whether
the Test: footer already ran fx test on a runnable test package (and treated
device fx_test aggregator suites as runnable via fx bazel test), and (2)
commit_body_crate_name_as_target flagged <crate_name>_test in the commit body
even when an explicit standalone test target with that exact name (e.g.,
rustc_test(name = "<crate_name>_test")) was defined in BUILD.bazel or BUILD.gn.

## Why not a one-off fix

Suppressing these warnings only for a single package would leave
commit_message_format firing false positives whenever a migration touches an
already-migrated helper package alongside a device test migration, or whenever a
library with a custom crate_name pairs with an explicit standalone
<crate_name>_test target.
