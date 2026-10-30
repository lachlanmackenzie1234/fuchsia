# Dual build explicit fx package test decomposition and dep parity

- **Learned from:** inner loop friction on CL Ic7e194751d0a1cbe2b335937e5f9d14891a9e7d0, patchset 2 (task `starnix-syscall-loop`)
- **Date:** 2026-10-29
- **Changed:** `checks/gn_dep_parity.sh`, `prompts/coder.md`

## Root cause

When migrating a dual-build test package with a custom manifest to an explicit
fx_component_manifest + fx_test_component + fx_package(test_components = ...) +
fx_test chain, bazel2gn generates fuchsia_component_manifest,
fuchsia_test_component, and fuchsia_test_package("<pkg>-package") { package_name
= "<pkg>-gn" } below the sentinel, and group("tests") depends on
":<pkg>-package" alongside ":bazel_tests". However, prompts/coder.md still
stated that bazel2gn ignores fx_component_manifest, fx_test_component, and
fx_package for tests and instructed coders to hand-write a
fuchsia_unittest_package above the sentinel, while gn_dep_parity.sh only
expanded newly added direct edges (new_direct - old_all) in reach() and did not
record fx_package(package_name, test_components), fx_test_component,
fx_unittest_component, or fx_packaged_binary aliases in bazel_test_aliases() or
traverse same-package test package roots.

## Why not a one-off fix

Exempting specific test package names or directories would leave other
dual-build migrations that decompose fuchsia_unittest_package or
fuchsia_test_package into explicit fx_package(test_components = ...) +
fx_test_component + fx_test targets vulnerable to false-positive
dep_label_substituted errors on group("tests") or same-named test packages and
conflicting instructions between prompts/coder.md and
prompts/playbooks/case2_dual_build.md.
