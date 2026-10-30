# Dual build skipped target rdep visibility and cross rule diff alignment

- **Learned from:** inner loop friction on a local task (task `starnix-vdso`)
- **Date:** 2026-10-27
- **Changed:** `checks/gn_attr_parity.sh`, `checks/visibility_audit.sh`, `tools/find_rdeps.sh`, `tools/run_checks.sh`

## Root cause

visibility_audit and find_rdeps treated every # @bazel2gn:skip Bazel target as
GN-only when scanning BUILD.gn callers, causing valid visibility grants to GN
consumers on dual-maintained targets (such as a skipped
alias/resource/loadable_module kept hand-written above the BAZEL2GN SENTINEL) to
be flagged as false_positive_rdep_visibility. In addition, gn_attr_parity lacked
INFO coverage for config() -> build_flags() ldflags/cflags comparisons involving
rebase_path() string concatenation, custom *_resource rules taking their target
label via src and mapping multi-branch GN outputs to Bazel dest, loadable_module
-> fx_cc_binary and basic_source_set -> fx_cc_library cross-rule mappings, and
newly added Bazel dependencies already synced below the BAZEL2GN SENTINEL in
BUILD.gn.

## Why not a one-off fix

Hardcoding exemptions for a single package or target name would fail on all
future dual-build migrations that pair # @bazel2gn:skip Bazel targets with
hand-maintained pre-sentinel GN targets, convert loadable_module or
basic_source_set targets to fx_cc_binary/fx_cc_library, package binaries via
transition resource rules using src/dest, or add header dependencies synced by
bazel2gn.
