# Correlated upstream ci cq failure rebase

- **Learned from:** cq failure on CL I13a4b9797326fcc4e9c660e87afe666cd1277850, patchset 5 (task `starnix-block-server`)
- **Date:** 2026-10-23
- **Changed:** `prompts/coder.md`

## Root cause

The coder prompt instructed rebasing onto origin/main only when CQ failed with
'Failed to rebase' or when cq_reachability reported an upstream merge conflict,
omitting CQ failures flagged by LUCI autocorrelator as correlated upstream CI
false rejections ('Correlated failure found in CI' / 'This failure may be a
false rejection' such as tree-wide image size budget regressions on the base
revision).

## Why not a one-off fix

Any migration change tested while origin/main has a transient CI regression can
fail CQ with a correlated false rejection; instructing the coder to rebase onto
origin/main on correlated upstream CI failures resolves this across all
migration packages without altering target build definitions.
