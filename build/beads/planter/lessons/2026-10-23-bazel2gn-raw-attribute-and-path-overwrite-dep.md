# Bazel2gn raw attribute and path overwrite dep alignment coverage

- **Learned from:** inner loop friction on a local task (task `starnix-fxfs-crypto-insecure`)
- **Date:** 2026-10-23
- **Changed:** `checks/gn_attr_parity.sh`, `checks/manifest.json`

## Root cause

When a migrated BUILD.bazel target preserves a GN-only dependency (such as the
//build/validate:non_production_tag assert_no_deps group tag or a GN-only
action/component dependency) in the synced BUILD.gn target via `#
@bazel2gn:raw_attribute:deps += [...]` or `# @bazel2gn:path_overwrite:...`,
`check_dep_alignment_info` in `checks/gn_attr_parity.sh` only checked
`unmatched_bazel` entries against `third_party_target_map.json` and
`feature_sets`/variable-expanded deps. It did not inspect
`@bazel2gn:raw_attribute:` / `@bazel2gn:path_overwrite:` annotations or the
dependencies preserved in the generated `BUILD.gn` target below `## BAZEL2GN
SENTINEL`, leaving `unmatched_gn_canons` uncovered and causing `review_diff` to
emit a false-positive `dep_alignment` warning.

## Why not a one-off fix

Hardcoding a single target directory or specific library name would fail
whenever any other Fuchsia package preserves
`//build/validate:non_production_tag` or another GN-only dependency in
`BUILD.gn` via `@bazel2gn:raw_attribute` or `@bazel2gn:path_overwrite`.
Extracting `@bazel2gn:raw_attribute` and `@bazel2gn:path_overwrite` dependency
annotations and checking dependencies preserved in the synced `BUILD.gn` target
inside `checks/gn_attr_parity.sh` resolves this class of `dep_alignment` false
positive across all dual-build migrations.
