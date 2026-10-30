# Visibility scoping and diffengine narrowing reconciliation

- **Learned from:** inner loop friction on a local task (task `starnix-modules-ipc-mem`)
- **Date:** 2026-10-27
- **Changed:** `checks/manifest.json`, `checks/visibility_audit.sh`

## Root cause

When migrating a GN target whose pre-migration BUILD.gn used a subtree wildcard
(such as //<prefix>/*) or stale visibility entry, find_rdeps.sh and
visibility_audit.sh compute actual reverse dependencies across the repository
and require narrowing visibility to actual callers (for example
//<prefix>:__pkg__ when only //<prefix> depends on the target, rejecting
//<prefix>:__subpackages__ with overbroad_subpackages_visibility ERROR).
However, diffengine (review_diff) inspects only the local BUILD.gn and
BUILD.bazel pair without repository-wide rdep knowledge and emits a blocking
visibility_narrowed WARNING whenever a GN /* scope is narrowed to :__pkg__ or
omitted, and visibility_audit.sh did not emit a matching INFO
visibility_narrowed coverage finding after verifying that all actual repository
reverse dependencies are covered.

## Why not a one-off fix

Hardcoding exemptions for individual module directories would fail on every
future package migration where a pre-migration BUILD.gn visibility wildcard
(such as //<dir>/*) is narrowed in BUILD.bazel to match actual reverse
dependencies as required by find_rdeps.sh and visibility_audit.sh.
