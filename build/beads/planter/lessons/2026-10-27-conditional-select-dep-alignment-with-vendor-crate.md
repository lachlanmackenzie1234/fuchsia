# Conditional select dep alignment with vendor crate mappings

- **Learned from:** inner loop friction on a local task (task `src-virtualization-tests-batch`)
- **Date:** 2026-10-27
- **Changed:** `checks/migration_sanity.sh`, `tools/run_checks.sh`

## Root cause

When a migrated Bazel target uses select() for platform-conditional
dependencies, bazel2gn generates if/else if blocks in BUILD.gn that each assign
deps = [...]. The diffengine parser overwrites earlier deps = [...] blocks with
the final branch in BUILD.gn while taking the union of select() branches in
BUILD.bazel, reporting dependencies from earlier branches as unmatched in Bazel.
Existing post-processing in run_checks.sh and migration_sanity.sh missed cases
where those conditional branches contained third-party Rust crate dependencies
(spelled //third_party/rust_crates/vendor:<crate> in BUILD.bazel vs
//third_party/rust_crates:<crate> in BUILD.gn) or multiple conditional deps =
assignments.

## Why not a one-off fix

Suppressing the finding for a single target directory would leave every future
migration that combines select() platform conditionals with third-party Rust
crate dependencies vulnerable to false-positive dep_alignment warnings.
