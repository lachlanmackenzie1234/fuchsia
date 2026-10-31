# Playbook: Case 2 Dual-Build (bazel2gn)

Use when the package retains `BUILD.gn`.
- **Sentinel & `fx bazel2gn`**: `BUILD.bazel` is the source of truth for all `bazel2gn`-convertible
  targets (libraries, binaries, FIDL, structured configs, `fx_component`, `fx_component_manifest`,
  `fx_compiled_manifest`, and tests). Delete their manual definitions above
  `## BAZEL2GN SENTINEL - DO NOT EDIT BELOW THIS LINE ##` in `BUILD.gn`, never hand-edit below the
  sentinel, and run `fx bazel2gn -d <dir>` after every `BUILD.bazel` edit.
- **GN-only attributes**: Targets with `exclude_toolchain_tags`, `concurrent_jobs`,
  toolchain-qualified `assert_no_deps`/`deps` labels, `rustenv`/`inputs` embedding generated files,
  or `configs -=` stay generated below the sentinel via dedicated attributes/tags
  (`disable_instrumentation`, `disable_build_flags`, `exclude_toolchain_tags=...`) or
  `# @bazel2gn:raw_attribute:` / `raw_overwrite` / `value_overwrite` annotations (see the coder
  prompt). Keep a target hand-written above the sentinel with `# @bazel2gn:skip` only when no
  annotation can express it.
- **Packages & components**: Non-test `fuchsia_package` / `fuchsia_package_with_single_component`
  targets migrate to `fx_package` / `fx_component` / `fx_component_manifest` / `fx_packaged_binary`
  in `BUILD.bazel` and are deleted from `BUILD.gn`. Standalone `fx_component` and `fx_component_manifest` targets
  with GN consumers sync below the sentinel via `bazel2gn`.
- **Dual-build tests & unified `# @bazel2gn:skip` rules**: Define tests in `BUILD.bazel`
  (`fx_unittest_package`, `fx_unittest_component`, custom-`.cml` `fx_package(test_components = ...)`
  + `fx_test`, or `host_*_test`; for Go, move `*_test.go` from `go_library.sources` into
  `host_go_test`/`go_test` `srcs` with `embed = [":<lib>"]`) without `# @bazel2gn:skip` so
  `bazel2gn` generates the GN copies (`<package_name>-gn`) below the sentinel. Never add
  `# @bazel2gn:skip` to a test rule or attribute (`with_unit_tests`, `with_host_unit_tests`,
  `test_deps`) unless one of the **5 unified test skip conditions** in the coder prompt holds:
  (1) target under test is skipped in `BUILD.bazel`, (2) test package uses `resources`/`tools`/custom
  environments rejected by `bazel2gn` (skip on the package, paired with a hand-written `<pkg>-gn`
  package above the sentinel wrapping the generated test binary), (3) host test sets
  `data`/`test_data`/`test_label`, (4) unsupported test binary rule / cross-package `rustc_test` /
  pre-existing skip, or (5) human-confirmed dual-testing cost exception
  (`# Needs reviewer attention (bazel2gn skip): ...` on a non-SDK-library test).
- **Wiring tests**: Wire Bazel tests into an ancestor `BUILD.bazel` `test_suite` when one exists
  (else `bazel_test_suite("bazel_tests")` above the sentinel), and list all generated GN test copies
  in `group("tests")` above the sentinel (host copies as `:<name>($host_toolchain)`).
- Starnix unit tests (`test_type = "starnix"` or `.cml` including
  `//src/sys/test_runners/starnix/unit_test.shard.cml`) MUST move to Bazel (starnix `kernel/core`,
  `kernel/vdso` and `unit_test.shard.cml` have Bazel targets; ASan exclusion stays in GN):
  `fx_packaged_binary` (`binary_name` = `program.binary` without `bin/`) + `fx_component_manifest`
  (`component_name` = old GN package name; `includes` = the `.cml` shards) +
  `fx_test_component(test_type = "starnix", deps = [<packaged bin>, ...])` + `fx_package` +
  `fx_test` (`max_log_severity = "ERROR"` only if GN had it). Add `"//src/starnix/kernel/vdso"` to
  `fx_test_component(deps)` ONLY when the test binary transitively links `//src/starnix/kernel/core`
  (GN propagates the vDSO through `core`). In BUILD.gn delete the hand-written test package and its
  comment; `group("tests")` gets `deps = [ ":<pkg>-package" ]` and
  `if (!is_asan) { deps += [ ":bazel_tests" ] }` (`# TODO(https://fxbug.dev/42059075): Re-enable starnix in these environments.`).
  No `Build-Only: true` (CQ must run `<pkg>` and `<pkg>-gn`). Keep a test in GN only if its own deps
  lack a Bazel target (e.g. `//zircon/system/ulib/trace-test-utils`).
