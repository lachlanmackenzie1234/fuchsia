# Bazel2gn test package and component translation parity

- **Learned from:** inner loop friction on CL If360d6314b1adc3c4597bbbdcaea551770e5b024, patchset 8 (task `src-sys-run-test-suite`)
- **Date:** 2026-10-28
- **Changed:** `checks/gn_attr_parity.sh`, `checks/gn_dep_parity.sh`, `checks/gn_target_type_parity.sh`, `tools/run_checks.sh`

## Root cause

When migrating a GN test package or custom-manifest unit test component to
fx_package(test_components = ...) + fx_test and fx_component_manifest +
fx_test_component, fx_test claims the original test name in BUILD.bazel while
bazel2gn translates the fx_package (typically named <name>-package) into
fuchsia_test_package("<name>-package") and fx_test_component into
fuchsia_test_component below the BAZEL2GN SENTINEL. gn_dep_parity treated the
original test package name as still existing in GN because fx_test had that name
in BUILD.bazel and flagged group("tests") depending on :<name>-package as
dep_label_substituted; gn_target_type_parity flagged fuchsia_unittest_component
becoming fuchsia_test_component as gn_target_type_changed without emitting
template_parity INFO coverage; and gn_attr_parity omitted sentinel_targets from
template_parity coverage and queried PATH planter rather than the running
/proc/<pid>/exe daemon.

## Why not a one-off fix

Any dual-build Fuchsia package whose unit or integration test requires a custom
.cml manifest must unroll fx_component_manifest + fx_test_component +
fx_package(name = "<test>-package", test_components = [...]) + fx_test(name =
"<test>") in BUILD.bazel and reference :<test>-package from group("tests") in
BUILD.gn after running fx bazel2gn, so gn_dep_parity, gn_target_type_parity, and
gn_attr_parity must recognize these bazel2gn test-package and test-component
translations across all packages.
