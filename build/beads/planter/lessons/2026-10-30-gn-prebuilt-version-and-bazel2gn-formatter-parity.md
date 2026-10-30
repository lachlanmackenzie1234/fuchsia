# Gn prebuilt version and bazel2gn formatter parity

- **Learned from:** cq failure on CL Ie636d3ad28da83a3f434495f4cd193f07d2953b4, patchset 6 (task `starnix-module-erofs`)
- **Date:** 2026-10-30
- **Changed:** `checks/gn_format.sh`, `checks/manifest.json`

## Root cause

When origin/main advances across a GN CIPD roll in manifests/jiri.lock (and
manifests/toolchain) that changes `gn format` output (such as adding empty lines
as separators between top-level target blocks), a worktree whose HEAD has not
yet rebased onto origin/main (because `git merge-tree --write-tree origin/main
HEAD` has no textual merge conflicts) or whose gitignored
`prebuilt/third_party/gn/<platform>/gn` binary was not updated still formats
`BUILD.gn` with the older GN binary during `fx bazel2gn` and `shac check`,
causing `gn_format` and `verify_bazel2gn` to fail when Gerrit CQ evaluates the
change on top of origin/main; `checks/gn_format.sh` previously only inspected
the working-tree `manifests/jiri.lock` rather than also checking
`origin/main:manifests/jiri.lock` when `origin/main` is ahead of `HEAD`.

## Why not a one-off fix

Re-running `fx bazel2gn` or formatting `BUILD.gn` in a single target directory
without checking the GN revision pinned on both `HEAD` and `origin/main` leaves
any other worktree or migration task branched before a GN roll vulnerable to the
same `gn_format` and `verify_bazel2gn` CQ failures.
