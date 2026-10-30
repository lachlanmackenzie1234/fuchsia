# Outbound bazel dependency and shard visibility verification

- **Learned from:** inner loop friction on a local task (task `starnix-modules-misc`)
- **Date:** 2026-10-27
- **Changed:** `checks/visibility_audit.sh`

## Root cause

The static visibility_audit check only audited whether targets defined inside
candidate BUILD.bazel files granted visibility to their reverse dependencies,
without checking the outbound direction: whether external in-tree Bazel targets
and exported files (such as cml_shard targets or exports_files(.shard.cml)
entries referenced in fx_component_manifest includes or target deps) in
untouched packages actually grant visibility to the newly migrated package. As a
result, run_checks.sh --skip-build passed with zero errors when a migrated
BUILD.bazel unrolled an fx_component_manifest referencing visibility-restricted
shards in untouched packages, deferring the Bazel visibility failure until full
build verification.

## Why not a one-off fix

Fixing only a single package's BUILD.bazel or hardcoding specific shard labels
would not prevent future migrations from referencing visibility-restricted
in-tree Bazel targets, cml_shard definitions, or exports_files entries in
untouched packages and passing all fast static checks before failing in Bazel
analysis.
