# Dual build test package parity and stale bazel2gn host tool refresh

- **Learned from:** inner loop friction on a local task (task `starnix-module-kgsl`)
- **Date:** 2026-10-29
- **Changed:** `checks/build_verification.sh`, `checks/gn_dep_parity.sh`

## Root cause

First, in build_verification.sh, when the Ninja host binary <build
dir>/host_*/bazel2gn was older than //build/tools/bazel2gn sources or the
Bazel-built bazel-bin/.../bazel2gn binary (which fx bazel2gn builds and executes
directly), build_verification.sh attempted to refresh it only via `fx build
--host //build/tools/bazel2gn:bazel2gn`. Whenever gn_build failed during
regenerator.py (for instance, when a parallel slice added a load from
@fuchsia_build_info//:args.bzl before the integration pass exported the GN build
argument in //build/bazel/BUILD.gn), `fx build --host
//build/tools/bazel2gn:bazel2gn` also aborted in regenerator.py, leaving <build
dir>/host_*/bazel2gn stale so `fx host-tool shac check` (verify_bazel2gn) ran an
outdated bazel2gn binary that lacked `fx_package(test_components = [...])`
support and falsely reported BUILD.gn out of sync with BUILD.bazel. Second, in
gn_dep_parity.sh,gn_test_package_reached needed to normalize hyphen/underscore
stem variants and follow cm_label edges when resolving decomposed dual-build
fuchsia_test_package and fuchsia_unittest_package targets wired into
group("tests").

## Why not a one-off fix

Fixing only a single package directory would leave every future dual-build
migration that decomposes unit test packages into fx_package(test_components =
[...]) + fx_test or triggers a regenerator failure before shac_check vulnerable
to false-positive verify_bazel2gn drift errors from stale <build
dir>/host_*/bazel2gn binaries and false-positive dep_label_substituted findings
on group("tests").
