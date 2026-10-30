# Rust crate name hyphen normalization alignment

- **Learned from:** inner loop friction on CL I2b03e2dd4bd43176b2629473098a3678ac28346a, patchset 1 (task `starnix-modules-usb-dev-devpts`)
- **Date:** 2026-10-27
- **Changed:** `checks/gn_attr_parity.sh`, `checks/manifest.json`

## Root cause

In //build/rust/rustc_*.gni, GN normalizes hyphens to underscores when computing
crate_name and output_name from name or output_name (via
string_replace(_crate_name, "-", "_")), while Bazel @rules_rust rejects hyphens
in crate_name and requires underscores directly. When a GN Rust target specified
a hyphenated name or output_name and the migrated BUILD.bazel target specified
the underscore-normalized crate_name, review_diff reported an
attribute_alignment warning (`output_name ([<hyphenated>]) != crate_name
([<underscored>])`) because gn_attr_parity.sh only emitted `added
crate_name=<val> in Bazel` and did not recognize hyphen-to-underscore
equivalence between GN name/output_name/crate_name and Bazel crate_name.

## Why not a one-off fix

Hardcoding a single target or crate name would leave every other Fuchsia Rust
library, binary, or test target whose GN `name` or `output_name` contains
hyphens vulnerable to the same false-positive `output_name != crate_name`
warning during migration. Normalizing hyphens to underscores generally in
`rust_crate_name_alignment_notes` and `is_benign_alignment_clause` inside
`checks/gn_attr_parity.sh` resolves this class of false positive across all Rust
targets.
