# Starnix and test type device test package migration parity

- **Learned from:** human review on CL I0e4ea39756c11f616d3ffeef2826561c52a17cb8, patchset 2 (task `starnix-modules-usb-dev-serial`)
- **Date:** 2026-10-29
- **Changed:** `checks/commit_message_format.sh`, `checks/cq_reachability.sh`, `checks/manifest.json`, `checks/migration_sanity.sh`

## Root cause

The static checks (`migration_sanity.sh`, `cq_reachability.sh`, and
`commit_message_format.sh`) allowed GN test packages with `test_type` (such as
`test_type = "starnix"`) or test binaries using `exclude_toolchain_tags` to
remain hand-written in `BUILD.gn` above `## BAZEL2GN SENTINEL` with obsolete `#
Bazel test packaging blocked:` comments claiming `//src/starnix/kernel/vdso`,
`exclude_toolchain_tags`, or `unit_test.shard.cml` blocked Bazel packaging:
`GN_TEST_PACKAGE_BLOCKERS` still listed `test_type` even when `bazel2gn`
converts `fx_package(test_components = ...)` + `fx_test` +
`fx_test_component(test_type = ...)`, `GN_ONLY_ATTR_RE` treated translatable
`exclude_toolchain_tags` on the test binary as a package blocker,
`is_resolved_packaging_blocker` only rejected `_solib_` runfiles comments, and
`commit_message_format.sh` did not reject `Build-Only: true` on CLs migrating
`test_type = "starnix"` test packages.

## Why not a one-off fix

Fixing only a single Starnix module or dependency directory leaves every future
package with a `test_type` device test package or `exclude_toolchain_tags` test
binary able to bypass `gn_test_package_not_migrated` and
`unpackaged_rust_device_test` by citing resolved
vDSO/`exclude_toolchain_tags`/`test_type` blockers or adding `Build-Only: true`;
updating `migration_sanity.sh`, `cq_reachability.sh`, and
`commit_message_format.sh` enforces full test-package migration and CQ test
execution across all packages.
