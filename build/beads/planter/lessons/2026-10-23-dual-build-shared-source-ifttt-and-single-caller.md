# Dual build shared source ifttt and single caller cross area visibility

- **Learned from:** human review on CL I13a4b9797326fcc4e9c660e87afe666cd1277850, patchset 8 (task `starnix-block-server`)
- **Date:** 2026-10-23
- **Changed:** `checks/manifest.json`, `checks/shared_config_reuse.sh`, `checks/visibility_audit.sh`, `tools/find_rdeps.sh`

## Root cause

First, shared_config_reuse did not verify bidirectional LINT.IfChange /
LINT.ThenChange comments when a dual-build package retains a non-test library,
staticlib, or fuzzer target above the BAZEL2GN SENTINEL in BUILD.gn that
compiles the same multi-file source list as a migrated library in BUILD.bazel.
Second, recommend_area_rollup_visibility in find_rdeps.sh and
visibility_audit.sh rolled up a single-caller group in another area into a
depth-2 //src/<area>:__subpackages__ wildcard whenever the caller path had depth
> 2, and visibility_audit.sh skipped the cross-area depth-2 wildcard check
whenever allow_broad_rollup was true for other multi-caller subtrees.

## Why not a one-off fix

Fixing only the single target package leaves other dual-build packages with
retained GN staticlib/variant/fuzzer targets vulnerable to silent source and
dependency list drift between BUILD.gn and BUILD.bazel, and allows any target
that uses an area rollup for its home subsystem to grant overbroad depth-2
//src/<area>:__subpackages__ visibility to unrelated areas where only a single
package depends on the target.
