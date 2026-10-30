# Visibility scoping of workspace-level external repository build file overlays (new local repository *.BUILD.bazel)

- **Learned from:** inner loop friction on CL Ib89bc0c5d6f1f51513c8b1bc0418daaccc39183f, patchset 3 (task `starnix-module-perfetto-consumer-perfetto`)
- **Date:** 2026-10-22
- **Changed:** `prompts/reviewers/target_parity.md`

## Root cause

The target_parity seat applied the rdep-based narrowing rules (find_rdeps
per_target_recommended_visibility) to targets in a central new_local_repository
build_file overlay. Callers reach those targets through @<repo>//... labels,
which find_rdeps cannot see, so the reviewer demanded narrowing back to the
first caller's :__pkg__. That contradicted human review asking for broad
visibility on the central overlay.

## Why not a one-off fix

Any migration that moves a third-party overlay for a shared external repository
into a central //build/bazel or //tools package will hit the same rdep-blind
narrowing demand. Fixing only this one overlay's visibility would leave every
future overlay migration stuck in the same conflict between the reviewer and
human feedback.
