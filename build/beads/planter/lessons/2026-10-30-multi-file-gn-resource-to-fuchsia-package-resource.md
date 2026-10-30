# Multi file gn resource to fuchsia package resource group alignment

- **Learned from:** inner loop friction on a local task (task `starnix-modules-ipc-mem`)
- **Date:** 2026-10-30
- **Changed:** `checks/gn_attr_parity.sh`, `checks/migration_sanity.sh`

## Root cause

In GN, resource() accepts multiple files in sources with template placeholders
in outputs (such as '{{source_file_part}}'), whereas Bazel's resource() rule
enforces allow_single_file = True on src and requires multi-file resources to
use fuchsia_package_resource_group(srcs = [...], dest = '...', basename_only =
True). Neither checks/gn_attr_parity.sh nor checks/migration_sanity.sh recorded
INFO attribute_alignment coverage for the resource ->
fuchsia_package_resource_group rule mapping or the outputs vs dest attribute
comparison, leaving review_diff to emit false-positive attribute_alignment
warnings.

## Why not a one-off fix

Multi-file GN resource() targets using '{{source_file_part}}' in outputs occur
across many Fuchsia test and component packages; recognizing the resource ->
fuchsia_package_resource_group rule and outputs/dest mapping in
gn_attr_parity.sh and migration_sanity.sh prevents false-positive review_diff
warnings across all future package migrations.
