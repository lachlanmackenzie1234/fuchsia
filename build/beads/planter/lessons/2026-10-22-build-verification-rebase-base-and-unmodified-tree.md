# Build verification rebase base and unmodified tree handling

- **Learned from:** inner loop friction on a local task (task `starnix-core-tests-bench-fuchsia`)
- **Date:** 2026-10-22
- **Changed:** `checks/build_verification.sh`

## Root cause

When a checkout was reset or rebased onto origin/main from a prior local branch
(or when HEAD sits directly on origin/main because a package is blocked by an
unmigratable dependency chain), PLANTER_CHANGE_BASE was no longer an ancestor of
HEAD~1. As a result, build_verification.sh failed to normalize change_base,
treating all upstream commits between the old base and origin/main (including
upstream shac.textproto version rolls and other packages' BUILD files) as task
changes. This invalidated the build-verification fingerprint cache, disabled the
shac version-mismatch fallback because shac.textproto appeared in changed_set,
and ran full product builds on an unmodified tree where transient RBE
credential-helper deadlines caused false build_failure_gn_build errors.

## Why not a one-off fix

Fixing or skipping only a single target directory would not prevent stale
pre-rebase PLANTER_CHANGE_BASE commits, unmodified blocked trees, or transient
RBE credential-helper timeouts from breaking build_verification across future
GN-to-Bazel migration tasks.
