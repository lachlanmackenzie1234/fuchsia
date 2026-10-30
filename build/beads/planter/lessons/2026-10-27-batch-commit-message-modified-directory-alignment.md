# Batch commit message modified directory alignment

- **Learned from:** human review on CL Iaa2e4e4a06b03bca4b85c475637090a69a5c8251, patchset 6 (task `src-connectivity-lib-batch`)
- **Date:** 2026-10-27
- **Changed:** `checks/commit_message_format.sh`, `checks/manifest.json`

## Root cause

When a multi-directory batch migration task had several directories already
migrated on main or dropped during rebase so the commit only modified a subset
of PLANTER_TARGET_DIR, commit_message_format.sh still treated the unmodified
first entry of PLANTER_TARGET_DIR as the primary migrated directory, did not
verify that the commit subject path or 'Also migrated in this change:' bullets
had modified files in the commit diff, and allowed commit body narration about
untouched batch directories.

## Why not a one-off fix

Manually editing the commit message of a single batch CL does not prevent future
multi-directory batch migrations or rebased CLs from retaining stale subject
directories, 'Also migrated in this change:' bullets, or body commentary
referencing unmodified packages.
