# Dual-build check conflicts: pre-existing skip annotations in packages touched only for visibility, and fuchsia package with single component split into fuchsia package plus a bazel2gn-generated fuchsia component

- **Learned from:** inner loop friction on a local task (task `starnix-hello-starnix`)
- **Date:** 2026-10-26
- **Changed:** `checks/gn_dep_parity.sh`, `checks/gn_target_type_parity.sh`, `checks/migration_sanity.sh`

## Root cause

migration_sanity judged every # @bazel2gn:skip in each changed dual-build
package, including ones that pre-existed at the change base in packages edited
only for visibility, so the coder could not satisfy both it and the
unrelated-changes rule. gn_target_type_parity and gn_dep_parity did not know
that bazel2gn now translates fx_component/fx_component_manifest, so they
reported the equivalent split of fuchsia_package_with_single_component into
fuchsia_package + same-package fuchsia_component (the shape the duplicate-target
remediation asks for) as a template change and a sibling-label substitution.

## Why not a one-off fix

Any migration that grants visibility on an already-migrated upstream package
with owner-added skips, or that moves a single-component Fuchsia package to
fx_package/fx_component in a dual-build package, hits the same contradictory
findings. Disputing them one directory at a time would leave the checks blocking
every later Starnix, component or example package migration.
