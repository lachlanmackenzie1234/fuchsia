# Premature gn test package private component migration

- **Learned from:** human review on CL Idaf0d2169fa7a11bfa0100aee465d521e3b2f98d, patchset 1 (task `pkg-tests-omaha-client`)
- **Date:** 2026-10-29
- **Changed:** `checks/migration_sanity.sh`, `prompts/coder.md`

## Root cause

prompts/coder.md instructed migrating helper components listed in a test
package's deps to fx_component(testonly = True) without distinguishing
components consumed by a Bazel fx_package or external callers from
package-private driver/helper components whose sole consumer is a GN-blocked
test package kept above ## BAZEL2GN SENTINEL, and checks/migration_sanity.sh
only flagged # @bazel2gn:skip duplicate component wrappers rather than
unskipped, unreferenced fx_component/fx_component_manifest/fx_packaged_binary
chains that force cross-package .shard.cml visibility edits.

## Why not a one-off fix

Many integration test directories across Fuchsia retain a GN-only
fuchsia_test_package above ## BAZEL2GN SENTINEL until realm dependency
components migrate to Bazel. Fixing only one package would leave future partial
test migrations free to emit unreferenced Bazel fx_component_manifest wrappers
and modify unrelated external BUILD.bazel shard visibility lists, needlessly
pulling cross-subsystem OWNERS into self-contained test binary migrations.
