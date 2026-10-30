# Dual build test platform compatibility path overwrite and ifttt hygiene

- **Learned from:** human review on CL If876caff81e2bcda299c8a695eb6eed71aa22cfa, patchset 4 (task `starnix-ext4-lib`)
- **Date:** 2026-10-23
- **Changed:** `checks/bazel_minimality.sh`, `checks/commit_message_format.sh`, `checks/gn_attr_parity.sh`, `checks/gn_dep_parity.sh`, `checks/shared_config_reuse.sh`

## Root cause

First, commit_message_format.sh did not flag unresolved local task names (e.g.
`CL <task-slug>`) in `Depends on:` blocks and previously rejected brief `Migrate
//<dir> from GN to Bazel.` body descriptions as redundant title restatements.
Second, bazel_minimality.sh allowed `# @bazel2gn:path_overwrite://<dep_pkg>` on
`//<dep_pkg>:<target>` even when `//<dep_pkg>:<target>` was already defined in
`<dep_pkg>/BUILD.gn` with visibility covering the caller package and
`//<dep_pkg>` was only a pure forwarding group, while gn_dep_parity.sh emitted a
false `dep_label_removed` INFO finding and gn_attr_parity.sh omitted
`dep_alignment` coverage when a forwarding group was replaced by its direct
target. Third, bazel_minimality.sh did not check that Fuchsia-only `rustc_test`
targets attached via `crate` to cross-platform libraries set
`target_compatible_with = ["@platforms//os:fuchsia"]` so `bazel2gn` wraps the
generated GN `rustc_test` in `if (is_fuchsia)`. Fourth, shared_config_reuse.sh
only enforced bidirectional `LINT.IfChange`/`LINT.ThenChange` comments on
dual-maintained `build_flags()` and library source lists, missing `#
@bazel2gn:skip` `resource`/`fuchsia_package_resource_group` and
`fx_unittest_package` targets duplicated above `## BAZEL2GN SENTINEL` in
`BUILD.gn`.

## Why not a one-off fix

Fixing only the single package leaves all future dual-build migrations
vulnerable to leaving unresolved local task names in commit messages, retaining
redundant `# @bazel2gn:path_overwrite` annotations for already-migrated GN
targets, omitting `target_compatible_with = ["@platforms//os:fuchsia"]` on
Fuchsia-only `rustc_test` targets attached to cross-platform libraries, and
omitting bidirectional `LINT.IfChange`/`LINT.ThenChange` markers and `testonly =
True` on skipped test resource groups.
