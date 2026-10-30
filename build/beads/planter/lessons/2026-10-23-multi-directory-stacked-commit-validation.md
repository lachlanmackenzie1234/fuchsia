# Multi directory stacked commit validation

- **Learned from:** cq failure on CL I1b7877d67dc8935cd5510603ec5304b01555ed81, patchset 4 (task `starnix-module-kgsl`)
- **Date:** 2026-10-23
- **Changed:** `checks/commit_message_format.sh`

## Root cause

The previous patchset failed in CQ because it had been rebased onto origin/main
as a single commit without its unmerged parent migration stack (before
missing_bazel_dependency_package was added to bazel_minimality.sh). When the
full multi-commit stack and multi-directory target scope were restored,
commit_message_format.sh read only PLANTER_TARGET_DIR instead of
PLANTER_TARGET_DIRS during per-directory check invocations, causing each
secondary target directory in PLANTER_TARGET_DIRS to be treated as the sole
primary directory and falsely triggering commit_body_redundant_also_migrated
warnings on valid 'Also migrated in this change:' bullets.

## Why not a one-off fix

Fixing only a single package or manually editing one commit message would leave
commit_message_format.sh broken for every future multi-package migration task
where RunCheckTool invokes checks once per directory in PLANTER_TARGET_DIRS.
