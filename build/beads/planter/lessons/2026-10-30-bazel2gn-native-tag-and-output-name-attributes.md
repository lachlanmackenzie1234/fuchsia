# Bazel2gn native tag and output name attributes

- **Learned from:** human review on CL Ib4d1eb86f918e01ab151bfcdef3f0b96d530c1c6, patchset 2 (task `starnix-modules-fs`)
- **Date:** 2026-10-30
- **Changed:** `checks/bazel_minimality.sh`, `checks/cq_reachability.sh`, `checks/gn_attr_parity.sh`, `checks/migration_sanity.sh`, `prompts/coder.md`

## Root cause

The coder prompt and GN attribute parity remediation examples still showed `#
@bazel2gn:raw_attribute:exclude_toolchain_tags = [ ... ]` ahead of native `tags
= ["exclude_toolchain_tags=<tag>"]` tags, and `checks/bazel_minimality.sh` only
rejected `# @bazel2gn:raw_attribute:output_name` on Rust test targets without
flagging `# @bazel2gn:raw_attribute:exclude_toolchain_tags` on Rust binary,
dylib, and test targets when `bazel2gn` natively translates
`exclude_toolchain_tags=` tags (and also emitted false-positive
`cl_scope_exceeded` errors when a multi-directory task migrated only its
assigned `PLANTER_TARGET_DIRS`).

## Why not a one-off fix

Updating only the individual filesystem module `BUILD.bazel` files would leave
future dual-build Rust migrations using raw
`@bazel2gn:raw_attribute:exclude_toolchain_tags` directives instead of native
`tags = ["exclude_toolchain_tags=..."]` attributes and would not prevent
`cl_scope_exceeded` from rejecting multi-directory tasks that migrate only their
assigned target directories.
