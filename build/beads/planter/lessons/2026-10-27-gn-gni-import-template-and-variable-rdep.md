# Gn gni import template and variable rdep visibility resolution

- **Learned from:** inner loop friction on CL I4fda514490455dbe58280c229090f8d9f704d911, patchset 1 (task `src-security-tests-pkg-test-batch`)
- **Date:** 2026-10-27
- **Changed:** `checks/manifest.json`, `checks/visibility_audit.sh`, `tools/find_rdeps.sh`, `tools/manifest.json`

## Root cause

When a .gni file defines a GN template() or top-level shared dependency variable
referencing a migrated target and a BUILD.gn in another directory imports that
.gni and instantiates the template or variable, GN evaluates visibility in the
importing BUILD.gn's package rather than the .gni's directory. Without full .gni
import expansion across working-tree, origin/main, and fallback searches for
both template calls and exported top-level variables, find_rdeps and
visibility_audit miss the importing caller package and misclassify its required
visibility entry as a false-positive rdep.

## Why not a one-off fix

Any Fuchsia subsystem that factors reusable test or component assembly logic
into .gni templates or shared dependency lists imported by sibling or parent
BUILD.gn packages hits this exact GN visibility enforcement rule, so find_rdeps
and visibility_audit must resolve .gni imports to their instantiating BUILD.gn
packages generically across the entire tree.
