# Gn template relative import caller visibility discovery

- **Learned from:** cq failure on CL I4fda514490455dbe58280c229090f8d9f704d911, patchset 1 (task `src-security-tests-pkg-test-batch`)
- **Date:** 2026-10-27
- **Changed:** `checks/visibility_audit.sh`, `tools/find_rdeps.sh`

## Root cause

When a .gni template references a dual-built target (such as a compiled_action
host tool or component dependency), GN evaluates the template and enforces
target visibility in the package of the BUILD.gn file that imports the .gni and
invokes the template, not in the directory where the .gni file resides. Both
tools/find_rdeps.sh and checks/visibility_audit.sh missed such BUILD.gn callers
for three reasons: (1) discover_referencing_build_files only grepped for the
target package path and did not expand matched .gni files to discover BUILD.gn
files importing them; (2) gni_template_map matching only checked literal
source-root-absolute '//path/to/file.gni' strings and missed directory-relative
import('sub/file.gni') statements; and (3) compute_package_rdeps in
checks/visibility_audit.sh executed an early 'continue' when the target package
path did not appear literally in a BUILD.gn file before evaluating
gni_call_matches, causing visibility_audit both to miss the uncovered caller and
to reject subpackage visibility covering the caller as overbroad.

## Why not a one-off fix

Manually widening visibility in a single BUILD.bazel file would leave find_rdeps
and visibility_audit blind to every other GN template across the tree that is
imported via a relative path or invoked from a parent/sibling BUILD.gn file,
causing visibility_audit to keep recommending incomplete visibility lists and
rejecting valid parent/subpackage visibility grants.
