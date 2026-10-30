# Gn local template expansion unit test parity reconciliation

- **Learned from:** inner loop friction on CL If360d6314b1adc3c4597bbbdcaea551770e5b024, patchset 8 (task `src-sys-run-test-suite`)
- **Date:** 2026-10-28
- **Changed:** `checks/gn_attr_parity.sh`

## Root cause

When a file-local GN template(...) wraps a Rust library rule such as
rustc_library(target_name) with with_unit_tests = true and conditional feature
forwarding like features = [] if (defined(invoker.features)) { features +=
invoker.features }, static diffengine analysis parses the unexpanded inner
template rule body and records literal invoker.features or template-scoped
feature expressions on the paired Bazel library target. It then emits a
false-positive review_diff/unit_test_parity error claiming the Bazel rustc_test
target is missing crate_features such as [invoker.features] or features only
enabled by other template invocations. Although gn_attr_parity already
reconciled diffengine findings for file-local GN template expansions across
template_parity, unmatched_target, dep_alignment, attribute_alignment, and
dropped_comment, its target-message matcher only recognized messages starting
with Target or GN target (missing Unit tests of "<target>") and did not
reconcile benign unit_test_parity findings on expanded file-local template
targets.

## Why not a one-off fix

Suppressing unit_test_parity for a single package or hardcoding a specific
template name would fail whenever any other Fuchsia package defines a file-local
GN template wrapping rustc_library or rustc_binary with conditional
invoker.features and with_unit_tests = true and expands those invocations into
concrete Bazel library and unit-test targets. Extending gn_attr_parity's
file-local template reconciliation to parse Unit tests of "<target>" messages
and verify that any reported missing crate_features are either unexpanded
invoker.* expressions or already satisfied by comparing the concrete Bazel
library target's crate_features against its unit test target's crate_features
prevents this entire class of false-positive unit_test_parity errors across the
repository.
