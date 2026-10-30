# Stale gate record after shared machinery sync

- **Learned from:** human review on CL Ifcab9bc2dbc8d4b619b21d6964f14d5a1763c285, patchset 2 (task `starnix-modules-usb-dev`)
- **Date:** 2026-10-30
- **Changed:** `checks/gn_attr_parity.sh`, `checks/manifest.json`

## Root cause

The upload gate evaluated the patchset against a prior gate record recorded
before the shared check machinery was synced with origin/main; the current check
scripts and coder instructions already cover inner sanitizer guards on
bazel_test_suite dependencies in group("tests"), landed stacked dependencies,
and clear disputed_findings requirements when run_checks.sh passes.

## Why not a one-off fix

Gate record staleness after a shared machinery update affects any migration task
whose patchset was validated prior to the machinery sync, so the resolution is
re-running run_checks.sh against the updated shared machinery rather than
changing target-specific rules.
