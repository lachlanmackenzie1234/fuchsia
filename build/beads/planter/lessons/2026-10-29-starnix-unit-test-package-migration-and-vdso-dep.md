# Starnix unit test package migration and vdso dep parity

- **Learned from:** human review on CL Ica59b69e1dfc25d0a561e9209a693e7da1c678a5, patchset 2 (task `starnix-modules-misc`)
- **Date:** 2026-10-29
- **Changed:** `checks/cq_reachability.sh`, `checks/migration_sanity.sh`, `prompts/playbooks/case2_dual_build.md`

## Root cause

While `migration_sanity` and `cq_reachability` already rejected obsolete `#
Bazel test packaging blocked:` comments for `test_type = "starnix"` packages,
three gaps remained: (1) `migration_sanity` only recognized a GN test package as
a Starnix unit test when `BUILD.gn` explicitly set `test_type = "starnix"`,
missing Starnix unit test packages whose `.cml` manifest includes
`//src/sys/test_runners/starnix/unit_test.shard.cml` without `test_type =
"starnix"` in `BUILD.gn`; (2) `case2_dual_build.md` and `migration_sanity`
unconditionally instructed adding `"//src/starnix/kernel/vdso"` to
`fx_test_component(deps = [...])` for all Starnix tests instead of only when the
test binary transitively links `//src/starnix/kernel/core`, and no check
verified `//src/starnix/kernel/vdso` presence/absence or `if (!is_asan)` gating
on `:bazel_tests` in `group("tests")`; and (3) `cq_reachability` only inspected
the exact paths in `PLANTER_TARGET_DIRS` without expanding modified child
packages under a parent target directory.

## Why not a one-off fix

Fixing only the individual Starnix module directories in this CL would leave
future Starnix unit test migrations vulnerable to misclassifying `.cml`-based
Starnix test packages as standard `fx_unittest_package` targets, adding
unnecessary `//src/starnix/kernel/vdso` dependencies to leaf-crate tests that do
not link `//src/starnix/kernel/core` (or omitting the vDSO on tests that do),
omitting `if (!is_asan)` around `:bazel_tests` in `group("tests")`, or skipping
`cq_reachability` checks on child packages when a parent directory is specified
as the migration target.
