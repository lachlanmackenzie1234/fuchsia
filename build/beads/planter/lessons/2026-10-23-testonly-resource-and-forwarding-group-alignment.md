# Testonly resource and forwarding group alignment coverage

- **Learned from:** inner loop friction on CL If876caff81e2bcda299c8a695eb6eed71aa22cfa, patchset 4 (task `starnix-ext4-lib`)
- **Date:** 2026-10-23
- **Changed:** `checks/gn_attr_parity.sh`

## Root cause

When a pre-migration GN resource or test-data target omitted testonly = true
even though it is only consumed by tests, adding testonly = True on the migrated
Bazel fuchsia_package_resource_group or host_test_data_* target caused
review_diff to combine 'added testonly=True in Bazel' with 'outputs (...) !=
dest (...)' into a single attribute_alignment warning that gn_attr_parity.sh did
not emit INFO coverage for.

## Why not a one-off fix

Hardcoding a single package or resource target name would fail to cover any
other migrated package where a GN resource, resource_group, or host_test_data
target lacked testonly = true in the base BUILD.gn and sets testonly = True
alongside dest/outputs mapping in BUILD.bazel.
