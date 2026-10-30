# Unamended working-tree bazel2gn sync changes and fx component/fx component manifest convertibility parity

- **Learned from:** inner loop friction on CL I0f132cfe7d44f413c1b7e1fa9f7632455d04d455, patchset 1 (task `starnix-modules-usb-dev-nanohub`)
- **Date:** 2026-10-24
- **Changed:** `checks/migration_sanity.sh`

## Root cause

When HEAD was already the migration commit (PLANTER_CHANGE_BASE != HEAD), static
checks inspected working-tree files on disk without verifying that git status
had no unamended working-tree modifications relative to HEAD, and
migration_sanity did not include fx_component and fx_component_manifest in
CONVERTIBLE_BAZEL_RULES or ignore multi-label comment blocks that also cite an
unmigrated dependency.

## Why not a one-off fix

Amending BUILD.gn only in the current target directory would leave future
dual-build migrations vulnerable to passing run_checks.sh while leaving
regenerated BUILD.gn outputs unstaged in the working tree instead of amended
into HEAD.
