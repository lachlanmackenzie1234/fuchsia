# Per attribute alignment info coverage for decomposed and retained targets

- **Learned from:** inner loop friction on CL I17f3707a50e56003344900597ab3497f24de8821, patchset 1 (task `starnix-hello-starnix`)
- **Date:** 2026-10-26
- **Changed:** `checks/gn_attr_parity.sh`

## Root cause

emit_rust_target_attr_alignment_info and related Target Attributes INFO
generators in checks/gn_attr_parity.sh only emitted multi-attribute
semicolon-joined permutations for res_prefix_notes (such as ldflags) and
preserved extra_attrs (such as disable_syslog_backend, emit_tool_path, and
exclude_toolchain_tags), omitting standalone single-attribute INFO entries
needed to match per-attribute review_diff attribute_alignment warnings.

## Why not a one-off fix

Hardcoding target names or rule-specific exceptions for a single package would
leave all other migrations that decompose custom GN executable/resource
templates, action rules, or retained pre-sentinel targets vulnerable to
false-positive per-attribute review_diff attribute_alignment warnings whenever a
target has multiple uncarried GN attributes.
