# Sanitizer gated bazel test suite and gn condition parity

- **Learned from:** inner loop friction on a local task (task `starnix-module-erofs`)
- **Date:** 2026-10-30
- **Changed:** `checks/test_suite_layout.sh`

## Root cause

When a migrated test binary carries exclude_toolchain_tags for sanitizer
variants (asan/hwasan), Bazel ignores those GN toolchain tags, requiring the GN
bazel_test_suite entry in group("tests") to be gated by if (!is_asan) locally
whenever an ancestor test_suite is unconditional or sibling packages use
per-package bazel_test_suite targets. In test_suite_layout.sh,
pkg_exclude_tag_conds matched GN list assignments for exclude_toolchain_tags but
required a quote immediately after '=' and therefore missed Bazel's tags =
["exclude_toolchain_tags=asan"] syntax when no generated GN test target was
present.

## Why not a one-off fix

Fixing or waiving the finding for a single package would leave every other
package that migrates sanitizer-excluded unit tests (especially packages using
Bazel exclude_toolchain_tags=<tag> tags without a generated GN test target or
packages in subtrees with per-package bazel_test_suite targets) vulnerable to
false-positive per_package_bazel_test_suite and unrequested_gn_condition errors.
