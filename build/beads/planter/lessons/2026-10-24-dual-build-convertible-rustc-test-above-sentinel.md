# Dual build convertible rustc test above sentinel

- **Learned from:** human review on CL I733d05125173d968298e788a9a4ec995ea38f014, patchset 13 (task `starnix-core`)
- **Date:** 2026-10-24
- **Changed:** `checks/manifest.json`, `checks/migration_sanity.sh`

## Root cause

migration_sanity.sh treated any hand-written GN rustc_test above ## BAZEL2GN
SENTINEL without a matching Bazel target name as an informational
gn_test_kept_above_sentinel finding, failing to check whether all of the test's
Rust crate and C/C++ library dependencies already have BUILD.bazel targets (with
any GN-only attributes or non-crate resource/data deps expressible via #
@bazel2gn:raw_attribute: annotations), and
check_stale_missing_bazel_dep_comments required all labels in a comment block to
be Bazel-buildable before flagging stale missing-Bazel-dependency claims.

## Why not a one-off fix

Fixing only the single package directory would allow future dual-build Rust
library migrations to leave convertible rustc_test targets and duplicated
top-level source/dependency list variables above ## BAZEL2GN SENTINEL whenever
the test package has GN-only attributes or mixed migrated and unmigrated
resource dependencies.
