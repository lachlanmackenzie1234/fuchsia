# Rdep discovery git grep fallback and shac prebuilt version skew

- **Learned from:** inner loop friction on CL I462bbd3688dc180405084e19fa258593b0abe48a, patchset 1 (task `starnix-dm-verity`)
- **Date:** 2026-10-22
- **Changed:** `checks/build_verification.sh`, `checks/visibility_audit.sh`, `tools/find_rdeps.sh`

## Root cause

First, checks/visibility_audit.sh batch-discovered reverse-dependency build
files exclusively via `rg` and silently swallowed `FileNotFoundError` when `rg`
was absent from PATH, leaving `matched_files` empty (unlike
`tools/find_rdeps.sh`, which fell back to `git grep`) and causing valid caller
packages to be flagged as `false_positive_rdep_visibility`. Second, when a
checkout or worktree rebased onto `origin/main` after an upstream
`min_shac_version` bump in `shac.textproto` without updating CIPD prebuilts
(`prebuilt/tools/shac/shac`), `checks/build_verification.sh` treated `shac:
min_shac_version specifies unsupported version` as a blocking
`build_failure_shac_check` error instead of verifying dual-build `bazel2gn
--check_only` sync directly and reporting the prebuilt version skew as `INFO`.

## Why not a one-off fix

Hardcoding visibility exceptions for a single package or skipping `shac_check`
unconditionally would leave every future migration task vulnerable to empty
reverse-dependency sets whenever `rg` is unavailable in the environment and to
false `build_failure_shac_check` blockers whenever `origin/main` rolls
`min_shac_version` ahead of the local checkout's CIPD `shac` prebuilt.
