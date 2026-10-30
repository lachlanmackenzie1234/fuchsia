# Subpackage exports files and crate inherited sources parity

- **Learned from:** inner loop friction on a local task (task `starnix-core-fs-fuchsia`)
- **Date:** 2026-10-27
- **Changed:** `checks/gn_attr_parity.sh`, `checks/manifest.json`

## Root cause

When migrating a child directory into its own BUILD.bazel subpackage that
exports source files to an enclosing parent package via exports_files(), the
parent BUILD.bazel replaces package-relative subpackage paths (<sub_dir>/<file>)
with cross-package labels (//<pkg>/<sub_dir>:<file>). In
checks/gn_attr_parity.sh, check_proto_and_cross_rule_alignment_info only
normalized //pkg/sub:file to //pkg/sub/file without also mapping it to the
parent-package-relative path sub/file present in the base BUILD.gn, and did not
construct an intermediate source set inheriting crate = ':<lib>' srcs/hdrs
separately from compile_data/data (nor inherit compile_data/data in
check_compile_data_inputs_info), causing review_diff to report false-positive
missing_sources errors on both the parent library and its crate-linked unit
test.

## Why not a one-off fix

Suppressing missing_sources only for one specific package or test target would
fail whenever any other parent package's sources are split across a newly
migrated child BUILD.bazel subpackage using exports_files() or whenever a
rustc_test target inherits both cross-package sources and compile_data inputs
through crate = ':<lib>'.
