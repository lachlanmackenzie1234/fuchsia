# Local gn template expansion and unreferenced variant test alignment

- **Learned from:** inner loop friction on CL I9665eb5ba5b0b2bbfbde82ebcc1403a9202fd833, patchset 15 (task `starnix-core-net-leaves-fidl_fuchsia_net_neighbor_ext`)
- **Date:** 2026-10-22
- **Changed:** `checks/gn_attr_parity.sh`

## Root cause

When a pre-migration BUILD.gn defines a file-local template('<name>') wrapping
an inner target rule such as rustc_library_with_features(target_name),
review_diff's regex parser treats the inner target_name call inside the template
body as an unmatched top-level GN target and also flags any unreferenced
secondary variant unittest package that was never wired into the parent
group('tests'). gn_attr_parity recorded alignment coverage for the outer
template invocations but omitted the inner template rule's target_name
declaration and unreferenced secondary variant unittest packages.

## Why not a one-off fix

Multiple Fuchsia crates (especially dual-mode FIDL/FDomain extension crates)
define file-local GN helper templates wrapping rustc_library_with_features or
rustc_library; handling inner template target_name declarations and unreferenced
variant test packages generically in gn_attr_parity prevents false-positive
unmatched_target errors across all such migrations.
