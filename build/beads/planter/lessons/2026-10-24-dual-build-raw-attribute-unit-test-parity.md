# Dual build raw attribute unit test parity

- **Learned from:** inner loop friction on CL I7691d2fed42bf601f99469e4d3e57d074e384643, patchset 4 (task `starnix-module-perfetto-consumer-perfetto_consumer`)
- **Date:** 2026-10-24
- **Changed:** `checks/gn_attr_parity.sh`

## Root cause

checks/gn_attr_parity.sh only set has_decomposed_test_rule when an explicit
rustc_test or Bazel test rule existed in the package, missing dual-build Rust
libraries that preserve with_unit_tests = true (alongside GN-only attributes
like exclude_toolchain_tags or test_deps) on the bazel2gn-generated target via
@bazel2gn:raw_attribute while retaining their GN test package above the
sentinel. As a result, gn_attr_parity omitted the 'GN with_unit_tests decomposed
into explicit test target' permutation from its INFO attribute_alignment
coverage, leaving diffengine's attribute_alignment warning undeduplicated.

## Why not a one-off fix

Any dual-build Rust library whose unit test remains in GN (due to
exclude_toolchain_tags, unsupported test_type, or unmigrated test-only
dependencies) and preserves with_unit_tests = true via @bazel2gn:raw_attribute
triggers the same diffengine attribute_alignment warning unless gn_attr_parity
recognizes cur_top with_unit_tests preservation across all packages.
