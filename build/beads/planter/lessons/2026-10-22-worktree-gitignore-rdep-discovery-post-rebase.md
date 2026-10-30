# Worktree gitignore rdep discovery, post-rebase change base normalization, and multi-line GN dep alignment message parity

- **Learned from:** inner loop friction on CL Ib9c6d67d32874089943b31ed0306e4730e4b1767, patchset 1 (task `starnix-framebuffer-libs`)
- **Date:** 2026-10-22
- **Changed:** `checks/build_verification.sh`, `checks/migration_sanity.sh`, `checks/visibility_audit.sh`

## Root cause

First, visibility_audit.sh ran rg without --no-ignore-parent and returned an
empty set when rg exited with code 1; in a worktree under
.jiri_root/worktrees/<slot>, the parent checkout's .gitignore ignores
/.jiri_root/, causing rg to ignore the entire worktree and exit 1 without
falling back to git grep (whereas find_rdeps.sh fell back to git grep on exit
code 1). Second, when a migration branch is rebased onto origin/main,
PLANTER_CHANGE_BASE still points at the pre-rebase parent commit, so git diff
against PLANTER_CHANGE_BASE in build_verification.sh and visibility_audit.sh
includes upstream commits (such as shac.textproto version rolls, disabling
verify_bazel2gn_on_shac_version_mismatch). Third, migration_sanity.sh's
record_multiline_gn_dep_alignment_info omitted exact_cnt ('<N> exact match(es)')
from the INFO dep_alignment message, preventing deduplication of review_diff's
WARNING when gn format wraps public_deps/deps onto the next line.

## Why not a one-off fix

Hardcoding exemptions for specific libraries or packages would leave all future
dual-build migrations running in .jiri_root/worktrees/<slot> or rebasing onto
origin/main vulnerable to false-positive visibility_audit errors, false shac
version mismatch failures, and false-positive review_diff dependency alignment
warnings.
