# Dual build obsolete migration blocker comment removal parity

- **Learned from:** inner loop friction on a local task (task `starnix-core-tests-pkg`)
- **Date:** 2026-10-27
- **Changed:** `checks/gn_attr_parity.sh`

## Root cause

In checks/gn_attr_parity.sh, check_obsolete_gn_retention_comments_info returned
early whenever cur (the target in the updated BUILD.gn) was not None, which
skipped every dual-build target that stays in BUILD.gn (either generated below
the BAZEL2GN SENTINEL or retained above the sentinel with a -gn package_name
suffix). Additionally, its regex required the comment line to start immediately
with '# cannot migrate to Bazel' or '# kept in GN', missing comments where a
target or subject precedes 'cannot be migrated to Bazel' or where the comment
states that dependencies 'do not have Bazel targets yet' (which
migration_sanity's stale_missing_bazel_dep_comment check requires removing once
the dependencies have Bazel targets).

## Why not a one-off fix

Any dual-build GN target or test package that was previously deferred with a
pre-migration blocker comment ('<target> cannot be migrated to Bazel yet because
... do not have Bazel targets yet') will trigger false-positive review_diff
dropped_comment warnings when migrated in a follow-up change while its GN
counterpart remains in BUILD.gn. Extending
check_obsolete_gn_retention_comments_info in checks/gn_attr_parity.sh to cover
dual-build targets and missing-Bazel-target blocker phrasing resolves this
conflict across all packages.
