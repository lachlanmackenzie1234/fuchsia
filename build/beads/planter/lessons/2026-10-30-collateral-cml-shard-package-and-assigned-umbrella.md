# Collateral cml shard package and assigned umbrella child scoping

- **Learned from:** inner loop friction on CL Ica59b69e1dfc25d0a561e9209a693e7da1c678a5, patchset 2 (task `starnix-modules-misc`)
- **Date:** 2026-10-30
- **Changed:** `checks/bazel_minimality.sh`, `checks/manifest.json`

## Root cause

checks/bazel_minimality.sh treated any newly created BUILD.bazel outside
PLANTER_TARGET_DIRS and same-area dependency_dirs as an out-of-scope package
migration and counted it toward the 5-directory CL scoping limit, even when its
BUILD.gn was untouched and it only declared non-convertible
cml_shard/exports_files/expect_includes targets required for sandboxed cmc
compilation of an in-scope component manifest. In addition,
check_cl_scoping_guidelines() only exempted multi-directory tasks when
migrated_pkgs was a literal subset of target_dirs, failing when an assigned
target_dir was an umbrella group('tests') directory whose sole migrated package
lived in a child subdirectory and double-counting bazel2gn-regenerated BUILD.gn
lines.

## Why not a one-off fix

Exempting only a single shard directory path or specific task directories would
leave bazel_minimality failing with false-positive out_of_scope_package_modified
and cl_scope_exceeded errors whenever any future component or test migration
defines a missing cml_shard in a cross-area shard directory or migrates an
assigned batch containing an umbrella parent target directory.
