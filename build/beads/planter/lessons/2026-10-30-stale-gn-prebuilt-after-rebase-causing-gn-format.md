# Stale gn prebuilt after rebase causing gn format and bazel2gn drift

- **Learned from:** cq failure on CL Ica59b69e1dfc25d0a561e9209a693e7da1c678a5, patchset 4 (task `starnix-modules-misc`)
- **Date:** 2026-10-30
- **Changed:** `checks/gn_format.sh`

## Root cause

When a checkout is rebased onto a newer origin/main that rolled the CIPD
gn/gn/<platform> package in manifests/jiri.lock and manifests/toolchain (such as
a GN roll changing gn format block-separator rules), git rebase updates the
manifest files in git but leaves the gitignored
prebuilt/third_party/gn/<platform>/gn binary at the older revision. Because fx
bazel2gn, local fx host-tool shac check, and checks/gn_format.sh all executed
the stale local prebuilt/third_party/gn/<platform>/gn binary, local bazel2gn
runs generated BUILD.gn files without the new inter-target blank lines and local
gn_format and verify_bazel2gn checks passed, whereas CQ static-checks ran with
the manifest-pinned GN binary and failed both gn_format and verify_bazel2gn.

## Why not a one-off fix

Reformatting or re-syncing only the specific BUILD.gn files in this task is
insufficient because any migration checkout or worktree rebased across a GN
prebuilt roll in manifests/jiri.lock will continue invoking the stale local
prebuilt/third_party/gn/<platform>/gn during fx bazel2gn and local shac checks;
checks/gn_format.sh must systemically compare
prebuilt/third_party/gn/<platform>/gn --version against manifests/jiri.lock,
validate formatting with the manifest-pinned GN binary, and instruct the coder
to update prebuilt/third_party/gn/<platform> via cipd export before re-running
fx bazel2gn.
