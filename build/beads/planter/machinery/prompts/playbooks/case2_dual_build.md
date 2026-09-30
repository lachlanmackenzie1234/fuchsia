# Playbook: Case 2 Dual-Build (bazel2gn)

Use when the package keeps a BUILD.gn (it has GN-only constructs such as `bazel_test_suite`,
`group("tests")`, a test package Bazel cannot express yet, or GN dependents that must keep
working).
- For `bazel2gn`-convertible targets (libraries, standalone binaries, unit tests, FIDL),
  BUILD.bazel is the source of truth; never hand-edit BUILD.gn below `## BAZEL2GN SENTINEL` and
  run `fx bazel2gn -d <dir>` after every BUILD.bazel edit.
- Non-test `fuchsia_package`/`fuchsia_package_with_single_component`/`fuchsia_component` targets
  migrate to `fx_package`/`fx_component`/`fx_component_manifest`/`fx_packaged_binary` in
  BUILD.bazel and are deleted from BUILD.gn (`bazel2gn` does not convert them; omit the sentinel
  if BUILD.bazel only defines `fx_*` targets).
- Test packages follow "Migrating Tests" in the coder prompt. Rust unit tests: `rustc_*` with
  `with_host_unit_tests = True`, and the GN `fuchsia_unittest_package`/`fuchsia_test_package`
  wrapping the generated `:<name>_test` stays above the sentinel (Bazel cannot run Rust tests on
  device yet). C++ device tests, with `build/bazel/rules/testing/fx_test.bzl` in the checkout,
  become `fx_test_component` + `fx_package(test_components = ...)` + `fx_test` with
  `# @bazel2gn:skip`, exported by a hand-written `bazel_test_suite` above the sentinel that
  `group("tests")` lists.
