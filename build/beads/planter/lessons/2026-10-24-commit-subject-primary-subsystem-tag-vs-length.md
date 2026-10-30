# Commit subject primary subsystem tag vs length limit

- **Learned from:** inner loop friction on CL I892a9ed8a7c2db20713c5c51833aa8d855ae16d0, patchset 7 (task `starnix-kernel-manager`)
- **Date:** 2026-10-24
- **Changed:** `checks/commit_message_format.sh`, `checks/confidentiality_check.sh`, `checks/manifest.json`, `prompts/coder.md`

## Root cause

commit_message_format.sh extracted all bracketed tokens across git log subjects
without distinguishing primary subsystem tags from secondary sub-tags (such as
'[<primary>][<secondary>]'), cross-tree language/roll sweep tags, or reverted
commits, and prompts/coder.md did not clarify that resolved check conflicts must
not be listed in disputed_findings once run_checks.sh exits 0.

## Why not a one-off fix

Any deeply nested Fuchsia package whose git log contains multi-tag commits
('[<subsystem>][<subarea>]') or cross-tree roll commits will hit the same
65-character commit subject boundary and secondary-tag substitution unless git
log tag extraction separates primary subsystem tags from secondary and sweep
tags systemically.
