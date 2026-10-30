# Gni and bzl template caller rdep visibility resolution

- **Learned from:** inner loop friction on CL I25203d2901b1c8825a6dc822bf8d68db7a071b5f, patchset 1 (task `starnix-ext4-tests`)
- **Date:** 2026-10-24
- **Changed:** `checks/visibility_audit.sh`, `tools/find_rdeps.sh`

## Root cause

When a migrated target is referenced inside a .gni template or .bzl macro
(including via multi-line file-scope variable assignments or untracked build
files across repositories), GN and Bazel evaluate the template or macro in each
importing caller package's scope and enforce target visibility against the
caller package rather than only the defining .gni/.bzl directory.

## Why not a one-off fix

Any host tool, library, or resource target invoked through an imported .gni
template or .bzl macro across the tree requires visibility for the packages that
import and instantiate that template or macro; resolving .gni/.bzl importer
packages and untracked build files in both find_rdeps.sh and visibility_audit.sh
prevents false-positive and uncovered rdep errors across all migrations.
