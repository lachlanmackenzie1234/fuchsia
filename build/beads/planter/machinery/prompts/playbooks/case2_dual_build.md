# Playbook: Case 2 Dual-Build (bazel2gn)

Use when the package retains `BUILD.gn`.
- **Sentinel & `fx bazel2gn`**: `BUILD.bazel` is the source of truth for all `bazel2gn`-convertible
  targets (libraries, binaries, FIDL, structured configs, `fx_component`, `fx_component_manifest`,
  `fx_compiled_manifest`, `ffx_plugin`, and tests). Delete their manual definitions above
  `## BAZEL2GN SENTINEL - DO NOT EDIT BELOW THIS LINE ##` in `BUILD.gn`, never hand-edit below the
  sentinel, and run `fx bazel2gn -d <dir>` after every `BUILD.bazel` edit.
- **GN-only attributes**: Preserve `exclude_toolchain_tags`, `concurrent_jobs`, toolchain-qualified
  `assert_no_deps`/`deps`, `rustenv`/`inputs`, and `configs -=` below the sentinel via dedicated
  attributes/tags or `# @bazel2gn:raw_attribute:` / `raw_overwrite` / `value_overwrite` annotations
  (see `prompts/coder.md`).
- **Packages & components**: Migrate non-test `fuchsia_package` / `fuchsia_package_with_single_component`
  targets to `fx_package` / `fx_component` / `fx_component_manifest` / `fx_packaged_binary` in
  `BUILD.bazel` and delete them from `BUILD.gn`. Standalone `fx_component` and `fx_component_manifest`
  targets with GN consumers sync below the sentinel via `bazel2gn`.
- **Dual-build tests & `# @bazel2gn:skip` rules**: Define tests in `BUILD.bazel` without
  `# @bazel2gn:skip` so `bazel2gn` generates the GN copies (`<package_name>-gn`) below the sentinel.
  Never add `# @bazel2gn:skip` to a test rule or attribute unless one of the 5 unified test skip
  conditions in `prompts/coder.md` holds (Condition 5 `# Needs reviewer attention (bazel2gn skip):`
  may be proposed directly for Gerrit review without prior human sign-off, including when
  dual-running device test copies triggers `testsharder` `duplicatedShardNames` under `use_test_pilot`).
- **Wiring tests**: Wire Bazel tests into an ancestor `BUILD.bazel` `test_suite` when one exists
  (else `bazel_test_suite("bazel_tests")` above the sentinel), and list generated GN test copies
  in `group("tests")` above the sentinel (host copies as `:<name>($host_toolchain)`).
- **Starnix unit tests** (`test_type = "starnix"` or `.cml` including `starnix/unit_test.shard.cml`):
  Migrate to `fx_packaged_binary` + `fx_component_manifest` (`component_name` = old GN package name)
  + `fx_test_component(test_type = "starnix", deps = [...])` + `fx_package` + `fx_test` (adding
  `"//src/starnix/kernel/vdso"` to `fx_test_component(deps)` only when the test binary transitively
  links `//src/starnix/kernel/core`). In `BUILD.gn`, `group("tests")` gets `deps = [ ":<pkg>-package" ]`
  and `if (!is_asan) { deps += [ ":bazel_tests" ] }`.
