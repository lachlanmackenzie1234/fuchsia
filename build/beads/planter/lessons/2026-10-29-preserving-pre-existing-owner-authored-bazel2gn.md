# Preserving pre-existing owner-authored # @bazel2gn:skip packaging/component helpers during partial dual-build migrations

- **Learned from:** human review on CL I6885d1c5c4ceea8a2e1ddb42c98c15101ddc308b, patchset 2 (task `pkg-tests-pkg-resolver`)
- **Date:** 2026-10-29
- **Changed:** `checks/manifest.json`, `checks/migration_sanity.sh`

## Root cause

migration_sanity's skipped_convertible_target exempted pre-existing skips only
in ancillary packages outside PLANTER_TARGET_DIRS and only when the base
BUILD.gn already had a sentinel, so owner-added skips on component
manifests/resources already in the target package's BUILD.bazel were reported as
errors. That pushed the coder to rewrite those helpers and add Bazel components,
which the uploader rejected.

## Why not a one-off fix

Any package whose BUILD.bazel already holds owner-skipped manifests, components
or resources (kept in GN on purpose, e.g. because the GN twin carries content
Bazel lacks) hits the same false error when only its libraries and tests are
migrated. A per-directory fix would leave every such future partial migration
fighting the check.
