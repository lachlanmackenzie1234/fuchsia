# Bazel2gn raw attribute and decomposed unit test alignment

- **Learned from:** inner loop friction on CL I892a9ed8a7c2db20713c5c51833aa8d855ae16d0, patchset 6 (task `starnix-kernel-manager`)
- **Date:** 2026-10-22
- **Changed:** `checks/gn_attr_parity.sh`, `tools/run_checks.sh`

## Root cause

When a Rust target in BUILD.bazel maps its primary GN lint config via
lint_config, preserves GN-only conditional configs or helper variables in
BUILD.gn via @bazel2gn:raw_attribute, and decomposes with_unit_tests into an
explicit rustc_test target, gn_attr_parity only emitted INFO attribute_alignment
coverage when preserved_extra_attrs was non-empty and included extra-attribute
permutations, and standalone review_diff runs did not downgrade
@bazel2gn:raw_attribute-preserved target attribute differences.

## Why not a one-off fix

Suppressing the warning for a single package directory would leave every other
Rust target that combines lint_config, @bazel2gn:raw_attribute config
preservation, and explicit rustc_test decomposition vulnerable to false-positive
review_diff attribute_alignment warnings across the tree.
