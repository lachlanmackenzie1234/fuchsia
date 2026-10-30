# Host test data and decomposed unit test attribute parity

- **Learned from:** inner loop friction on CL Iff324bdcdeee736566ffcca4db9cc0c3d2d3bb7b, patchset 7 (task `starnix-container-lib`)
- **Date:** 2026-10-22
- **Changed:** `checks/gn_attr_parity.sh`, `checks/manifest.json`

## Root cause

gn_attr_parity did not record INFO coverage for three valid GN-to-Bazel
migration patterns flagged by review_diff: (1) cross-package Bazel file labels
(//<pkg>:<relpath>) in srcs/files_map matching GN source paths
(//<pkg>/<relpath>); (2) GN host_test_data targets mapped to Bazel
host_test_data_files or host_test_data_map where GN outputs is replaced by
dest_dir or files_map; and (3) GN rustc_* targets whose with_unit_tests is
decomposed into an explicit host_rustc_test or wrap_host_rust_test target and
whose test-only rustenv is moved to rustc_env on the test target.

## Why not a one-off fix

Any Fuchsia host test or Rust library migration that exports cross-package test
data files, migrates GN host_test_data to Bazel
host_test_data_files/host_test_data_map, or decomposes with_unit_tests into an
explicit host_rustc_test carrying test-only rustc_env will hit the same
false-positive missing_sources and attribute_alignment findings unless
gn_attr_parity recognizes these general AST mappings across all packages.
