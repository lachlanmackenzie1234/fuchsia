# Gn forward variables from scope and list variable dep alignment

- **Learned from:** inner loop friction on a local task (task `starnix-pseudo-fs`)
- **Date:** 2026-10-23
- **Changed:** `checks/gn_attr_parity.sh`, `checks/manifest.json`

## Root cause

The diff engine parses only literal dependency strings inside each GN target
block without expanding top-level GN scope variables referenced via
forward_variables_from(<scope>, ...) or top-level GN list variables referenced
in dependency attributes, while gn_attr_parity.sh only accounted for
rustc_library_with_features feature_sets and local template() definitions when
recording INFO dep_alignment coverage.

## Why not a one-off fix

Sharing common dependencies and attributes across multiple sibling targets in a
BUILD.gn file (such as standard and static variants of a library) via top-level
GN scope dicts with forward_variables_from() or top-level GN list variables is a
widespread pattern across Fuchsia packages, so gn_attr_parity.sh must parse and
resolve top-level GN scopes and list variables generally for all targets.
