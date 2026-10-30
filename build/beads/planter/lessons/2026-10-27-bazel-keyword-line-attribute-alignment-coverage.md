# Bazel keyword line attribute alignment coverage

- **Learned from:** inner loop friction on CL I733d05125173d968298e788a9a4ec995ea38f014, patchset 19 (task `starnix-core`)
- **Date:** 2026-10-27
- **Changed:** `checks/gn_attr_parity.sh`

## Root cause

When diffengine (review_diff) compares a GN attribute such as configs against a
mapped Bazel attribute such as lint_config or rustc_env (or reports an
added/changed Bazel attribute), it anchors the attribute_alignment finding on
the line number of the Bazel keyword argument node inside the rule call rather
than the rule call's start line (bt_line). However, gn_attr_parity.sh only
emitted its INFO attribute_alignment coverage entries at bt_line, and only
populated allowed_alignment_targets for rustc_library_with_features and
IDK/zx_library templates, so Planter's (category, file, line, message)
deduplication failed to match and suppress the review_diff WARNING on the
keyword argument line.

## Why not a one-off fix

Hardcoding a single package or target line number would fail for all other Rust,
C++, and component targets across the tree whose lint_config, rustc_env,
crate_name, exec_properties, or other mapped keyword arguments appear on a
different line from the rule call start; expanding INFO attribute_alignment
coverage in gn_attr_parity.sh onto the referenced Bazel keyword argument AST
line numbers (and including migrated rustc_* targets and config/env mismatch
clauses in diffengine alignment reconciliation) fixes the line-anchoring
mismatch across all packages.
