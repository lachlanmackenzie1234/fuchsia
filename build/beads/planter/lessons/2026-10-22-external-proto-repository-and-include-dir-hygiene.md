# External proto repository and include dir hygiene

- **Learned from:** human review on CL Ib89bc0c5d6f1f51513c8b1bc0418daaccc39183f, patchset 3 (task `starnix-module-perfetto-consumer-perfetto`)
- **Date:** 2026-10-22
- **Changed:** `checks/bazel_minimality.sh`, `checks/manifest.json`

## Root cause

The bazel_minimality check rejected edits to domain-specific .bzl macros outside
//build/bazel/ (such as //tools/protobuf:rust_proto_library.bzl) and central
new_local_repository build_file overlays under //tools/... as out-of-scope,
which pushed the migration into placing the third_party repository overlay build
file inside a consumer feature package and smuggling shell/execpath
substitutions into rust_proto_library include_dirs, while also failing to flag
redundant include_dirs on standalone single-file proto targets without
proto_deps.

## Why not a one-off fix

Fixing only a single consumer package leaves bazel_minimality blocking
legitimate updates to non-core .bzl macros loaded by migrated targets and
central third_party repository build_file overlays under //tools/..., and fails
to deterministically catch consumer-scoped new_local_repository build_file
paths, shell/execpath substitutions in rust_proto_library include_dirs, or
redundant include_dirs on standalone proto targets across future migrations.
