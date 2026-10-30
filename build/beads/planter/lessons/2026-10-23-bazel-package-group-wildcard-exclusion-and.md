# Bazel package group wildcard exclusion and resource group alignment

- **Learned from:** inner loop friction on CL If876caff81e2bcda299c8a695eb6eed71aa22cfa, patchset 2 (task `starnix-ext4-lib`)
- **Date:** 2026-10-23
- **Changed:** `checks/bazel_minimality.sh`, `checks/gn_attr_parity.sh`

## Root cause

First, check_missing_bazel_dependency_packages in checks/bazel_minimality.sh
treated Bazel package_group wildcard specifications ('//...' and '//<dir>/...')
as literal dependency package directories requiring a BUILD.bazel file at
'<dir>/.../BUILD.bazel'. Second, check_proto_and_cross_rule_alignment_info in
checks/gn_attr_parity.sh did not include ('resource',
'fuchsia_package_resource_group') in valid_cross_rules or emit INFO
attribute_alignment coverage when GN resource outputs (such as
'<dest>/{{source_file_part}}') map to Bazel fuchsia_package_resource_group dest.

## Why not a one-off fix

Any migration that touches a BUILD.bazel containing package_group(packages =
['//<dir>/...']) or migrates a multi-file GN resource() target with outputs =
['<dest>/{{source_file_part}}'] to fuchsia_package_resource_group(srcs = [...],
basename_only = True, dest = '<dest>') will hit the same false-positive
missing_bazel_dependency_package and review_diff attribute_alignment errors
unless handled generally in the static check scripts.
