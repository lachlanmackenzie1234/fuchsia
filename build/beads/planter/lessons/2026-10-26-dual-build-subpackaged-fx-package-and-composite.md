# Dual build subpackaged fx package and composite resource parity

- **Learned from:** inner loop friction on a local task (task `starnix-hello-starnix`)
- **Date:** 2026-10-26
- **Changed:** `checks/bazel_minimality.sh`, `checks/gn_attr_parity.sh`, `checks/migration_sanity.sh`

## Root cause

Three static-check blind spots caused false positives when migrating packages
that combine an fx_package, a retained GN test package with subpackages, and a
composite GN executable+resource template: (1) migration_sanity flagged a
retained GN fuchsia_package/fuchsia_package_with_single_component as
duplicate_gn_fuchsia_package even when another retained GN target above the
BAZEL2GN SENTINEL (such as a fuchsia_test_package listing it in subpackages)
depended on it because bazel2gn does not emit fx_package; (2) bazel_minimality
walked all labels in a BUILD.bazel file (including unreferenced testonly
targets) when checking whether an fx_package transitively packages a
rustc_dylib; and (3) gn_attr_parity omitted *_resource rules (such as
linux_resource) and split binary attributes (output_name, ldflags) from its
custom resource cross-rule INFO coverage.

## Why not a one-off fix

Any dual-build package migration where a retained GN test package subpackages a
newly migrated fx_package, where a BUILD.bazel file contains both a non-Rust
fx_package and a separate testonly Rust binary, or where a composite GN
executable+resource template splits into an fx_cc_binary plus a custom
*_resource rule will hit the exact same false-positive check failures unless the
checks track intra-file GN package references, scope fx_package rustc_dylib
reachability to targets reachable from fx_package, and recognize *_resource
cross-rule mappings.
