# Starnix module test packages now migrate to Bazel (vDSO and asan exclusion are solved)

- **Learned from:** human review of the Starnix module migrations (fxr/1930086 core tests, fxr/1918106 devpts)
- **Date:** 2026-10-29
- **Changed:** `prompts/playbooks/case2_dual_build.md`, Starnix module task descriptions

## Root cause

Earlier Starnix module migrations moved the `rustc_library` and `rustc_test`
targets to Bazel but kept the `fuchsia_unittest_package` test packages
hand-written in GN above the BAZEL2GN SENTINEL, with comments such as "Kept in
GN because starnix_core tests require `/pkg/data/libvdso.so` from the GN-only
`//src/starnix/kernel/vdso` target ... and `exclude_toolchain_tags` cannot be
expressed on Bazel `fx_test`". Both reasons are obsolete:

1. `//src/starnix/kernel/vdso` has a Bazel target (an alias visible to
   `//src/starnix:__subpackages__`). GN propagates the vDSO resource through
   `//src/starnix/kernel/core`'s deps; Bazel does not, so the Bazel test
   component must depend on `//src/starnix/kernel/vdso` directly.
2. The asan/hwasan exclusion is handled on the GN side: add the Bazel tests'
   `bazel_test_suite` to the GN `group("tests")` only `if (!is_asan)`
   (`is_asan` covers hwasan), as other Starnix test groups already do.

Also, bazel2gn now generates GN for explicit `fx_component_manifest` /
`fx_test_component` / `fx_package` tests, including the dual-build GN copy with
a `-gn` package name suffix, so no hand-written GN test package should remain.

## The pattern (copy from src/starnix/modules/devpts and src/starnix/kernel/core)

In BUILD.bazel, for each GN `fuchsia_unittest_package("<pkg>")` with
`manifest = "meta/<x>.cml"`, `deps = [ ":<lib>_test" ]`, `test_type = "starnix"`:

```bzl
fx_packaged_binary(
    name = "<lib>_test_packaged_bin",
    testonly = True,
    binary = ":<lib>_test",
    binary_name = "<binary named in meta/<x>.cml program.binary, without bin/>",
)

fx_component_manifest(
    name = "<pkg>-manifest",
    testonly = True,
    component_name = "<pkg>",  # keep the old GN component name / test URL
    includes = [
        "//sdk/lib/inspect:client.shard",
        "//sdk/lib/syslog:client.shard",
        "//src/sys/test_runners/starnix:unit_test.shard.cml",
    ],  # exactly the shards the .cml includes
    manifest = "meta/<x>.cml",
)

fx_test_component(
    name = "<pkg>-component",
    compiled_manifest = ":<pkg>-manifest",
    component_name = "<pkg>",
    test_type = "starnix",
    deps = [
        ":<lib>_test_packaged_bin",
        # In GN, //src/starnix/kernel/vdso is a resource() propagated through
        # //src/starnix/kernel/core's deps.
        "//src/starnix/kernel/vdso",
        # plus any other resources the GN package had
    ],
)

fx_package(
    name = "<pkg>-package",
    package_name = "<pkg>",
    test_components = [":<pkg>-component"],
)

fx_test(
    name = "<pkg>",
    max_log_severity = "ERROR",  # only if the GN test_specs had it
    package = ":<pkg>-package",
)
```

In BUILD.gn above the sentinel, delete the hand-written package and its
"cannot be migrated"/"kept in GN" comment, and use:

```gn
import("//build/bazel/bazel_test_suite.gni")

bazel_test_suite("bazel_tests") {
  target_tests = [ "//src/starnix/modules/<m>:<pkg>" ]
}

group("tests") {
  testonly = true
  deps = [ ":<pkg>-package" ]  # bazel2gn-generated GN copy (<pkg>-gn)

  # TODO(https://fxbug.dev/42059075): Re-enable starnix in these environments.
  # The GN copy's test binary is excluded from asan/hwasan variants with
  # exclude_toolchain_tags, which Bazel ignores.
  if (!is_asan) {
    deps += [ ":bazel_tests" ]
  }
}
```

Then run `fx bazel2gn -d <dir>`. Do not add `Build-Only: true` to these CLs:
CQ must run both `<pkg>` and `<pkg>-gn`.

Remaining genuine exception: a test package whose deps include a target that
really has no Bazel target (for example `perfetto_consumer_tests` needs
`//zircon/system/ulib/trace-test-utils`) stays in GN, with a comment naming
that dependency.

## Why not a one-off fix

Every Starnix module (about 40 directories) has the same
`fuchsia_unittest_package` + starnix test runner + vDSO shape, and the in-review
and rerun module tasks all copied the same obsolete justification.
