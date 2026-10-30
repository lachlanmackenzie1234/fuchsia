# Rust test crate name output name and gn list var parity

- **Learned from:** inner loop friction on CL I733d05125173d968298e788a9a4ec995ea38f014, patchset 13 (task `starnix-core`)
- **Date:** 2026-10-24
- **Changed:** `checks/bazel_minimality.sh`, `checks/gn_attr_parity.sh`

## Root cause

First, bazel_minimality unconditionally rejected `crate_name` on `rustc_test`
targets using `crate = ":<lib>"`, even when `crate_name` differs from `name`
(such as a `*_lib_test` or `*_bin_test` binary name matching the base GN
`rustc_test` `name`/`output_name`) so `bazel2gn` emits `output_name =
"<crate_name>"` in `BUILD.gn`. Second, `gn_attr_parity` did not resolve
top-level GN list variables or `crate = ":<lib>"` inherited
`srcs`/`compile_data`/`deps` when recording `missing_sources` and
`dep_alignment` coverage, and split `attribute_alignment` coverage across
separate blocks that omitted preserved `public_configs`, `edition`,
`package_name`/`component_name`, and `lint_config` replacements.

## Why not a one-off fix

Any dual-build Rust package that uses top-level GN list variables for
`sources`/`inputs`/`deps`, per-CPU `arch/${current_cpu}` `select()` expansions,
cross-package `compile_data` labels, `rustc_test(crate = ":<lib>")` inheriting
library `edition`/`srcs`/`deps`, or explicit `crate_name` on `rustc_test` to
preserve a custom GN `output_name` will hit these same false-positive findings
unless `bazel_minimality` and `gn_attr_parity` recognize these general GN/Bazel
patterns.
