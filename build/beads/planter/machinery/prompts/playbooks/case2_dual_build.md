# Playbook: Case 2 Dual-Build (bazel2gn)

Use when the package keeps a BUILD.gn (it has GN-only constructs such as test packages
`fuchsia_unittest_package`/`fuchsia_test_package`/`bootfs_test`, `group("tests")`, or GN
dependents that must keep working).
- For `bazel2gn`-convertible targets (libraries, standalone binaries, unit tests, FIDL),
  BUILD.bazel is the source of truth; never hand-edit BUILD.gn below `## BAZEL2GN SENTINEL` and
  run `fx bazel2gn -d <dir>` after every BUILD.bazel edit.
- Non-test `fuchsia_package`/`fuchsia_package_with_single_component`/`fuchsia_component` targets
  migrate to `fx_package`/`fx_component`/`fx_component_manifest`/`fx_packaged_binary` in
  BUILD.bazel and are deleted from BUILD.gn (`bazel2gn` does not convert them; omit the sentinel
  if BUILD.bazel only defines `fx_package` targets).
