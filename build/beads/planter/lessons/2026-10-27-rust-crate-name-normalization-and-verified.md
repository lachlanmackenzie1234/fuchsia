# Rust crate name normalization and verified visibility narrowing diff reconciliation

- **Learned from:** inner loop friction on a local task (task `starnix-module-ext4`)
- **Date:** 2026-10-27
- **Changed:** `checks/visibility_audit.sh`, `tools/run_checks.sh`

## Root cause

In tools/run_checks.sh, downgrade_check_cfg_diff_findings did not normalize
hyphens to underscores when comparing GN name/output_name against Bazel
crate_name (as //build/rust/rustc_*.gni does via string_replace(package_name,
"-", "_")), nor did it downgrade visibility_narrowed or test attribute_alignment
findings when review_diff ran standalone; additionally,
checks/visibility_audit.sh gated verified_vis_targets on target-body GN
visibility rather than recording all targets that pass visibility_audit rdep and
scoping checks.

## Why not a one-off fix

Hardcoding exemptions for a single Rust module or target directory would leave
all other migrated Rust libraries with hyphenated GN names and all targets whose
overbroad GN /* visibility is narrowed to verified caller-scoped Bazel
visibility vulnerable to false-positive review_diff warnings.
