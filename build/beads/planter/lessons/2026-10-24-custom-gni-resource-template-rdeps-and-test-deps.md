# Custom gni resource template rdeps and test deps alignment

- **Learned from:** inner loop friction on a local task (task `starnix-ext4-tests`)
- **Date:** 2026-10-24
- **Changed:** `checks/gn_attr_parity.sh`, `checks/manifest.json`, `checks/visibility_audit.sh`, `tools/find_rdeps.sh`

## Root cause

visibility_audit.sh and find_rdeps.sh classified any GN target above the
sentinel whose template name was not in the fixed CONVERTIBLE_GN_TEMPLATES set
as a GN-only nonconvertible target even when BUILD.bazel defined a migrated
resource or action target of the same name, causing callers in BUILD.gn to be
excluded from target_rdeps and flagged as false_positive_rdep_visibility.
Simultaneously, gn_attr_parity.sh only accounted for non_test_deps on libraries
(not test_deps merged into deps on standalone test targets) and lacked INFO
attribute_alignment coverage for custom .gni resource/action templates mapped to
fuchsia_package_resource_group with added dest.

## Why not a one-off fix

Exempting a single package or template name would leave every future migration
that converts a custom .gni resource/action wrapper template into
fuchsia_package_resource_group/compiled_action or migrates a GN rustc_test with
test_deps vulnerable to false_positive_rdep_visibility errors and review_diff
alignment warnings.
