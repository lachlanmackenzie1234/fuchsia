# Sequential Stack Rebase Conflict Detection Against Upstream origin/main

- **Learned from:** cq failure on CL Ib89bc0c5d6f1f51513c8b1bc0418daaccc39183f, patchset 5 (task `starnix-module-perfetto-consumer-perfetto`)
- **Date:** 2026-10-23
- **Changed:** `checks/cq_reachability.sh`

## Root cause

In `checks/cq_reachability.sh`, the upstream rebase conflict check ran a single
`git merge-tree --write-tree` comparing the tip working tree against
`origin/main` from their merge base instead of simulating step-by-step rebase of
each commit in a multi-commit stack (`merge_base..HEAD`), and its remediation
did not warn against `git rebase --onto origin/main HEAD~1` (which drops
unmerged parent migration commits in stacked branches).

## Why not a one-off fix

Rebasing only a single migration branch manually leaves future stacked
GN-to-Bazel migration chains vulnerable to intermediate stack commit rebase
conflicts on Gerrit CQ (`checkout|jiri patch`) or accidental dropping of parent
migration commits during `--onto` rebases; verifying sequential `git merge-tree
--write-tree` steps across `merge_base..HEAD` in `checks/cq_reachability.sh`
catches and guides stacked rebases across all packages.
