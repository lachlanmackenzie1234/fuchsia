# Bazel2gn raw attribute and test package skip parity

- **Learned from:** inner loop friction on CL I1b7877d67dc8935cd5510603ec5304b01555ed81, patchset 3 (task `starnix-module-kgsl`)
- **Date:** 2026-10-22
- **Changed:** `checks/gn_attr_parity.sh`, `checks/manifest.json`, `checks/migration_sanity.sh`, `tools/run_checks.sh`

## Root cause

When a Bazel genrule preserves GN action attributes (outputs, script, args,
depfile, hermetic_deps) below the sentinel via @bazel2gn:raw_attribute and
@bazel2gn:path_overwrite, diffengine iterates over a Go map of unmatched GN
attributes in randomized order, so emitting only a single fixed attribute order
in gn_attr_parity.sh failed to deduplicate non-deterministic permutations of
review_diff attribute_alignment warnings; similarly, fx_unittest_package
attributes unsupported by bazel2gn (such as test_type) require # @bazel2gn:skip
and a hand-written GN package above the sentinel.

## Why not a one-off fix

Suppressing or special-casing a single genrule or test package target would
leave all other migrations that map GN action() targets to Bazel genrule() with
@bazel2gn:raw_attribute or use unsupported fx_unittest_package attributes
vulnerable to non-deterministic review_diff attribute_alignment and
migration_sanity false positives.
