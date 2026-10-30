# Missing bazel dependency package verification

- **Learned from:** cq failure on CL If3b3814ef7f68257b4b5d7626adabb7239510ca8, patchset 12 (task `starnix-net-routes-ext`)
- **Date:** 2026-10-22
- **Changed:** `checks/bazel_minimality.sh`, `checks/manifest.json`

## Root cause

When a migrated BUILD.bazel references an in-tree dependency label `//<pkg>` or
`//<pkg>:<target>` whose package directory `<pkg>` has no `BUILD.bazel` or
`BUILD` file in the checkout (either because an unmigrated GN dependency was
copied directly into BUILD.bazel or because a stacked parent migration CL was
dropped when rebasing onto `origin/main` before the parent CL landed),
`bazel2gn` and all static checks (`run_checks.sh --skip-build`) previously
passed because `bazel2gn` translates labels syntactically without verifying that
referenced Bazel packages exist on disk. As a result, CQ failed during `fx
build` (`bazel cquery` for `tests.json`) with `ERROR: no such package '<pkg>':
BUILD file not found`.

## Why not a one-off fix

Fixing only the single target directory's missing dependency does not prevent
future migrations or rebased stacked CLs across the Fuchsia tree from
referencing in-tree `//<pkg>` dependency labels that lack a `BUILD.bazel` or
`BUILD` file on disk. Extending `checks/bazel_minimality.sh`
(`missing_bazel_dependency_package`) to deterministically verify that every
non-visibility in-tree `//<pkg>` label in migrated `BUILD.bazel` files resolves
to an existing `<pkg>/BUILD.bazel` or `<pkg>/BUILD` catches missing or dropped
dependency migrations immediately during static checks across all packages.
