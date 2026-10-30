# Pre-existing code-owner @bazel2gn:skip targets in the migrated package flagged as agent-introduced skips/duplicates

- **Learned from:** inner loop friction on CL I6885d1c5c4ceea8a2e1ddb42c98c15101ddc308b, patchset 2 (task `pkg-tests-pkg-resolver`)
- **Date:** 2026-10-29
- **Changed:** `checks/bazel_minimality.sh`, `checks/manifest.json`, `checks/migration_sanity.sh`

## Root cause

migration_sanity only exempted pre-existing skipped targets in ancillary
packages outside PLANTER_TARGET_DIRS whose base BUILD.gn already had a sentinel,
and bazel_minimality's cml_shard rule did not diff against the change base. A
target package whose BUILD.bazel already carried owner-written skipped
manifests/resources (base BUILD.gn without sentinel) therefore produced blocking
skipped_convertible_target, duplicate_target_above_sentinel and
unrolled_transitive_cml_shard_includes errors. That contradicted the coder rule
to preserve pre-existing owner skips and the human reviewer's request not to
touch them, which forced disputes.

## Why not a one-off fix

Any package that gets partially migrated after code owners hand-wrote skipped
Bazel twins (manifests, resources, resource groups) would hit the same
contradiction. Exempting one directory would leave every other such package
stuck between reviewer instructions and the checks, so the checks now compare
against the change base and only flag skips and raw shard includes that the
change itself introduces or modifies.
