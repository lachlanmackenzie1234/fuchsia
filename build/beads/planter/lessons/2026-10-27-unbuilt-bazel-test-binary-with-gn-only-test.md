# Unbuilt bazel test binary with gn only test package

- **Learned from:** human review on CL I110b0cec1edf55fd56c7800f10d3e4fbb587b407, patchset 2 (task `src_sys_time_httpsdate_time_source_tests`)
- **Date:** 2026-10-27
- **Changed:** `checks/cq_reachability.sh`, `checks/manifest.json`, `checks/migration_sanity.sh`

## Root cause

Both migration_sanity.sh (check_rust_unit_test_layout and
inspect_gn_target_deps) and cq_reachability.sh
(is_packaged_by_kept_gn_test_package and is_rust_device_test_packaged) exempted
a device rustc_test in BUILD.bazel from requiring an fx_unittest_package or
fx_test in BUILD.bazel whenever a GN test package kept above the BAZEL2GN
SENTINEL in BUILD.gn referenced the generated test binary, emitting an INFO
finding (gn_test_kept_above_sentinel) instead of an error. However, when the
enclosing test package remains in BUILD.gn without a corresponding
fx_unittest_package or fx_test in BUILD.bazel, nothing in the Bazel build graph
depends on the test binary in BUILD.bazel, so neither fx build nor presubmit
ever builds the test binary in Bazel (verify_bazel2gn only checks syntax
translation). Additionally, inspect_gn_target_deps in migration_sanity.sh only
scanned absolute // labels inside partially migrated dependency BUILD.gn files
and missed relative subpackage dependencies (such as <subpkg>:<target>) that
lack a BUILD.bazel.

## Why not a one-off fix

Fixing only a single test directory leaves migration_sanity.sh and
cq_reachability.sh accepting unpackaged device rustc_test targets in BUILD.bazel
across any future test package whose enclosing fuchsia_test_package or
fuchsia_unittest_package must stay in BUILD.gn due to unmigrated component or
group dependencies. Enforcing in migration_sanity.sh and cq_reachability.sh that
a device rustc_test in BUILD.bazel is rejected unless packaged by an
fx_unittest_package, fx_test, or standalone fx_component in BUILD.bazel (and
teaching inspect_gn_target_deps to detect relative subpackage blockers in
partially migrated dependencies) prevents unbuilt Bazel test binaries
systemically across all future migrations.
