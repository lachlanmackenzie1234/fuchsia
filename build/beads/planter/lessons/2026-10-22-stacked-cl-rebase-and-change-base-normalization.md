# Stacked cl rebase and change base normalization

- **Learned from:** cq failure on CL Ib89bc0c5d6f1f51513c8b1bc0418daaccc39183f, patchset 4 (task `starnix-module-perfetto-consumer-perfetto`)
- **Date:** 2026-10-22
- **Changed:** `checks/bazel_minimality.sh`, `checks/build_verification.sh`, `checks/visibility_audit.sh`

## Root cause

When a migration CL that is part of an unmerged stack (with Depends-on footers)
is re-checked after an upstream tip-of-tree CQ failure that was reverted on
origin/main, an automatic single-commit rebase onto origin/main drops the
unmerged parent commits from the stack, causing missing_bazel_dependency_package
on parent-CL packages. Furthermore, when the coder restores and rebases the full
parent CL stack onto origin/main, checks that only normalized
PLANTER_CHANGE_BASE to HEAD~1 when HEAD~1 was an ancestor of origin/main (or did
not normalize PLANTER_CHANGE_BASE at all, as in bazel_minimality.sh) treated all
parent commits in the stack as part of HEAD's diff.

## Why not a one-off fix

Addressing only a single package directory would leave every stacked migration
CL vulnerable both to losing unmerged parent commits during single-commit
rebases onto origin/main and to false-positive scope, visibility, and
build-verification findings across parent stack commits whenever
PLANTER_CHANGE_BASE points at origin/main below the stack.
