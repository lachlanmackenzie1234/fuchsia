# Deterministic planter binary discovery and single binary diff execution

- **Learned from:** human review on CL Ie4b75ae2bc2d1eeb829789a69daadc7a1a4bf253, patchset 2 (task `pkg-isolated-ota-env`)
- **Date:** 2026-10-29
- **Changed:** `checks/gn_attr_parity.sh`, `checks/migration_sanity.sh`, `checks/visibility_audit.sh`, `tools/run_checks.sh`

## Root cause

Prior evolutions duplicated diverging find_planter_bins/discover_planter_bins
helpers across tools/run_checks.sh, checks/gn_attr_parity.sh,
checks/visibility_audit.sh, and checks/migration_sanity.sh that scraped all
unrelated host PIDs in /proc, retained volatile /proc/<pid>/exe paths for
deleted inodes, probed world-writable /tmp/planter, and executed planter diff
across every discovered binary without breaking, multiplying check latency and
mixing findings from stale or unrelated planter processes.

## Why not a one-off fix

Binary discovery for review_diff and diffengine alignment runs on every
GN-to-Bazel migration task across all four check and runner scripts, so fixing
only one target directory or one check script leaves /proc scraping, /tmp
execution hazards, stale-binary finding pollution, and multi-binary diff fanout
active on all future migrations.
