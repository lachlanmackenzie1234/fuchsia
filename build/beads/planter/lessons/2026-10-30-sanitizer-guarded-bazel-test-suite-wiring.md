# Sanitizer guarded bazel test suite wiring

- **Learned from:** inner loop friction on a local task (task `starnix-module-framebuffer`)
- **Date:** 2026-10-30
- **Changed:** `checks/cq_reachability.sh`, `checks/gn_attr_parity.sh`, `checks/test_suite_layout.sh`, `prompts/coder.md`

## Root cause

When a migrated unit test binary sets exclude_toolchain_tags for asan/hwasan,
Bazel ignores exclude_toolchain_tags so the package's GN group("tests") must
gate bazel_test_suite("bazel_tests") with if (!is_asan) instead of wiring into
an unconditional ancestor test_suite. Previously, test_suite_layout and
cq_reachability only recognized sanitizer guards already written around
bazel_test_suite in BUILD.gn (missing exclude_toolchain_tags declared in
BUILD.bazel or deps -= guards), gn_attr_parity did not check BUILD.bazel for
exclude_toolchain_tags when validating inner if (!is_asan) guards in
group("tests"), and prompts/coder.md instructed agents to always wire into an
ancestor test_suite without mentioning the sanitizer-guard exception.

## Why not a one-off fix

Any Fuchsia package whose unit test binary uses exclude_toolchain_tags (such as
asan or hwasan exclusions) under an area with an unconditional ancestor
bazel_test_suite requires a package-local sanitizer-guarded
bazel_test_suite("bazel_tests") in BUILD.gn. Inferring required sanitizer guard
conditions from exclude_toolchain_tags in BUILD.bazel and BUILD.gn across
gn_attr_parity, test_suite_layout, cq_reachability, and prompts/coder.md
prevents false-positive unrequested_gn_condition and
per_package_bazel_test_suite errors across all future migrations.
