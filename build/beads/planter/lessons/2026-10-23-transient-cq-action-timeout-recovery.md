# Transient cq action timeout recovery

- **Learned from:** cq failure on CL If876caff81e2bcda299c8a695eb6eed71aa22cfa, patchset 5 (task `starnix-ext4-lib`)
- **Date:** 2026-10-23
- **Changed:** `prompts/coder.md`

## Root cause

The coder prompt listed rebase-onto-origin/main recovery triggers for rebase
conflicts and correlated CI failures, but omitted transient Bazel
process_wrapper action timeouts on untouched targets during CQ subbuilds.

## Why not a one-off fix

Transient Bazel action timeouts in CQ subbuilds can affect any migration CL
regardless of the package being migrated, so the coder workflow must handle them
generally rather than per target directory.
