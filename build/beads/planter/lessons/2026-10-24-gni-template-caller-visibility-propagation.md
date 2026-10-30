# Gni template caller visibility propagation

- **Learned from:** cq failure on CL I25203d2901b1c8825a6dc822bf8d68db7a071b5f, patchset 1 (task `starnix-ext4-tests`)
- **Date:** 2026-10-24
- **Changed:** `checks/manifest.json`, `checks/visibility_audit.sh`, `tools/find_rdeps.sh`, `tools/manifest.json`

## Root cause

When a migrated target (such as a host binary tool or helper library) is
referenced inside a GN template defined in a .gni file (or a Starlark macro in a
.bzl file), GN and Bazel evaluate the template body in the calling BUILD.gn or
BUILD.bazel package and enforce target visibility against each caller package
rather than only the directory where the .gni/.bzl file resides. Both
find_rdeps.sh and visibility_audit.sh previously attributed .gni/.bzl references
solely to the directory containing the .gni/.bzl file and excluded .gni/.bzl
matches from per_target_direct_rdeps, causing find_rdeps.sh to omit external and
internal packages that import and invoke the template, and causing
visibility_audit.sh both to miss uncovered template callers and to reject valid
visibility entries for those callers as false_positive_rdep_visibility.

## Why not a one-off fix

Manually widening visibility in a single BUILD.bazel file does not prevent
future migrations of tools or libraries referenced inside .gni templates or .bzl
macros from omitting external caller packages (or having visibility_audit.sh
reject those caller entries as false-positive reverse dependencies). Resolving
importing BUILD.gn and BUILD.bazel packages that invoke the referencing template
or macro in both find_rdeps.sh and visibility_audit.sh systemically discovers
and enforces visibility for all template-mediated callers across the repository.
