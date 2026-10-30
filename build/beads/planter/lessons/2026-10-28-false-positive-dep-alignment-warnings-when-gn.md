# False-positive dep alignment warnings when GN fuchsia * package targets with redundant library or crate deps are migrated to Bazel fx test, fx package, or fx unittest package rules

- **Learned from:** inner loop friction on a local task (task `starnix-core-fs-fuchsia`)
- **Date:** 2026-10-28
- **Changed:** `checks/gn_attr_parity.sh`, `checks/migration_sanity.sh`, `tools/run_checks.sh`

## Root cause

In GN, fuchsia_unittest_package and fuchsia_test_package targets sometimes list
library or Rust crate targets in deps alongside the test binary even though
library rlibs have no effect on package contents and are already in the test
binary's deps. In Bazel, fx_test, fx_package, and fx_unittest_package package
components, test binaries, and resources rather than accepting library deps
directly. Existing dep_alignment compatibility logic in migration_sanity.sh,
gn_attr_parity.sh, and run_checks.sh only covered fx_package targets with
components/resources or full GN dep lists, missing fx_test and
fx_unittest_package targets and non-local GN dep subsets after diffengine
resolves local test binary references.

## Why not a one-off fix

Suppressing the warning for a single test package directory would leave every
other GN test package migration that had redundant library or crate entries in
fuchsia_unittest_package or fuchsia_test_package deps blocked by the same
false-positive review_diff dep_alignment warning on fx_test or
fx_unittest_package.
