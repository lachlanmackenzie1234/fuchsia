# Bazel2gn exec properties and directive comment hygiene

- **Learned from:** human review on CL I733d05125173d968298e788a9a4ec995ea38f014, patchset 19 (task `starnix-core`)
- **Date:** 2026-10-27
- **Changed:** `checks/gn_attr_parity.sh`, `checks/migration_sanity.sh`, `prompts/coder.md`

## Root cause

First, prompts/coder.md and migration_sanity.sh still showed or suggested
stacked # @bazel2gn:raw_attribute: annotations for concurrent_jobs.rust_highmem
(_concurrent_jobs, test_configs, test_vars) on Rust targets even though bazel2gn
natively translates exec_properties = RBE_USE_LARGE_WORKER from
//build/bazel/rbe:rbe.bzl into _concurrent_jobs settings in GN, and
gn_attr_parity.sh did not record alignment coverage when test_configs/test_vars
map to exec_properties. Second, migration_sanity.sh did not require
explanatory/TODO comments on # @bazel2gn:raw_attribute:deps += [...] annotations
for GN-only dependencies or on fidl_library(enable_cpp = False) targets that
retain @rules_fuchsia SDK C++ shims for SDK-rules consumers.

## Why not a one-off fix

Updating only the single migrated Rust package or FIDL library would leave
future migrations emitting manual _concurrent_jobs / test_configs / test_vars
raw attributes instead of exec_properties = RBE_USE_LARGE_WORKER, attaching
unexplained GN-only deps via raw_attribute, or leaving enable_cpp = False and
@rules_fuchsia FIDL C++ shims undocumented (or unremoved when unused) across
other packages.
