# Dual build retained pre sentinel and unmodified bazel target parity

- **Learned from:** human review on CL Ia9c3ab5ceca122e93ce01f68ecea748c2dd34ebe, patchset 8 (task `starnix-crypt`)
- **Date:** 2026-10-22
- **Changed:** `checks/gn_attr_parity.sh`, `checks/manifest.json`

## Root cause

When a dual-build package retains GN-only test, test-component, or test-package
targets hand-written above the BAZEL2GN SENTINEL in BUILD.gn (because their test
dependencies have no Bazel build yet or use GN-only test realm templates), or
modifies BUILD.gn in a package whose BUILD.bazel already defines the
corresponding targets on disk without diff changes in BUILD.bazel, diff-based
review_diff emits unmatched_target warnings and errors that gn_attr_parity did
not record as INFO coverage.

## Why not a one-off fix

Hardcoding specific storage or starnix package paths would fail whenever any
other dual-build migration retains blocked GN test/component/package targets
above the BAZEL2GN SENTINEL or touches a BUILD.gn whose companion BUILD.bazel
already defines the migrated targets on disk; recording general AST-based INFO
unmatched_target coverage in gn_attr_parity handles all such packages across the
repository.
