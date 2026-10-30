# Starnix test package migration and upstream rebase detection

- **Learned from:** human review on CL I0f132cfe7d44f413c1b7e1fa9f7632455d04d455, patchset 3 (task `starnix-modules-usb-dev-nanohub`)
- **Date:** 2026-10-29
- **Changed:** `checks/cq_reachability.sh`, `checks/migration_sanity.sh`

## Root cause

When a migration branch was based on an older commit before upstream origin/main
landed the Bazel Starnix vDSO, test runner shard, or dependency BUILD.bazel
files, STARNIX_VDSO_BAZEL_AVAILABLE and bazel_buildable in migration_sanity.sh
and cq_reachability.sh only inspected the local working tree instead of also
checking origin/main, and cq_reachability.sh allowed packaged_in_gn_pkg to
suppress unpackaged_rust_device_test even when a rustc_test carried an obsolete
resolved '# Bazel test packaging blocked:' comment.

## Why not a one-off fix

Fixing only a single Starnix module directory would leave other branches based
on pre-rebase commits passing static checks with obsolete '# Bazel test
packaging blocked:' or 'cannot be migrated to Bazel yet' comments whenever their
dependencies or vDSO targets landed on origin/main, and would fail to guide
conflict resolution to drop in-CL dependency migrations that have already landed
upstream.
