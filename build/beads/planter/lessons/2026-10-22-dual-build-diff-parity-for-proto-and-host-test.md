# Dual build diff parity for proto and host test rules

- **Learned from:** inner loop friction on CL Ib89bc0c5d6f1f51513c8b1bc0418daaccc39183f, patchset 3 (task `starnix-module-perfetto-consumer-perfetto`)
- **Date:** 2026-10-22
- **Changed:** `checks/gn_attr_parity.sh`, `checks/manifest.json`

## Root cause

The diff engine reported false-positive blocking findings on migrated packages
using rust_proto_library, top-level GN variable references in sources restored
via @bazel2gn:path_overwrite, and host_py_test targets migrated from
python_host_test because gn_attr_parity.sh only parsed quoted literal strings in
GN sources lists and lacked INFO parity coverage for rust_proto_library
crate_root/include_dirs omissions and valid host-test cross-rule mappings.

## Why not a one-off fix

Ignoring these findings only for a single package would leave all future
migrations of packages with rust_proto_library targets, GN variable-referenced
sources with @bazel2gn:path_overwrite, or python_host_test-to-host_py_test
conversions blocked by identical review_diff errors and warnings.
