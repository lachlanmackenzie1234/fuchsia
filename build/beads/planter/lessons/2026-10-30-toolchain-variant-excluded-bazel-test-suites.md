# Toolchain-variant-excluded Bazel test suites, subtree-scoped ancestor test suite discovery, and unrolled test type attribute alignment

- **Learned from:** inner loop friction on a local task (task `starnix-modules-usb-dev`)
- **Date:** 2026-10-30
- **Changed:** `checks/cq_reachability.sh`, `checks/gn_attr_parity.sh`, `checks/manifest.json`, `checks/test_suite_layout.sh`

## Root cause

First, `gn_attr_parity.sh` (`check_unrequested_gn_conditions` and
`check_gn_conditions_with_host_tools`) treated any newly added inner `if (...)`
block inside `group("tests")` as an unrequested platform/toolchain restriction
on a retained GN target, even when the `if (...)` block (such as `if (!is_asan)
{ deps += [ ":bazel_tests" ] }`) only wires a newly added `bazel_test_suite`
target in a package whose GN test binary uses `exclude_toolchain_tags` (which
Bazel ignores). Second, `test_suite_layout.sh` (`area_suite`) returned any
ancestor `test_suite()` without checking whether that ancestor suite covers the
package's subtree (ignoring sibling per-package `bazel_test_suite` precedent and
sub-tree-only ancestor suites that `cq_reachability.sh` already checks via
`ancestor_suite_covers_subtree`) or whether the local `bazel_test_suite` is
gated by a toolchain/sanitizer `if (...)` guard that the ancestor
`bazel_test_suite` lacks. Third, `check_test_specs_log_severity_alignment_info`
in `gn_attr_parity.sh` only generated full-length
`itertools.permutations(extra_attrs)` instead of all non-empty subset lengths
`1..len(extra_attrs)`, missing single-attribute `review_diff` alignment notes
such as `test_type in GN not carried over to Bazel` when
`fuchsia_unittest_package` is unrolled into `fx_test_component(test_type = ...)`
+ `fx_package` + `fx_test(max_log_severity = ...)`.

## Why not a one-off fix

Hardcoding a single package directory or subsystem path would leave every other
Fuchsia package whose tests carry `exclude_toolchain_tags` (requiring a
conditional `bazel_test_suite` dependency in `group("tests")`), reside in a
subtree that uses per-package `bazel_test_suite` targets alongside an unrelated
sub-directory `test_suite()` in an ancestor `BUILD.bazel`, or unroll
`fuchsia_unittest_package(test_type = ...)` into `fx_test_component` + `fx_test`
vulnerable to the same contradictory `unrequested_gn_condition`,
`per_package_bazel_test_suite`, and `attribute_alignment` findings.
