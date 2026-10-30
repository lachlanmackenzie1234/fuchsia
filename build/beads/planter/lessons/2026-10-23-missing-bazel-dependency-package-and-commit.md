# Missing bazel dependency package and commit subject length gating

- **Learned from:** cq failure on CL I892a9ed8a7c2db20713c5c51833aa8d855ae16d0, patchset 7 (task `starnix-kernel-manager`)
- **Date:** 2026-10-23
- **Changed:** `checks/bazel_minimality.sh`, `checks/commit_message_format.sh`

## Root cause

First, when an unmerged parent CL in a migration stack was dropped by a
single-commit rebase, `check_missing_bazel_dependency_packages` in
`checks/bazel_minimality.sh` only referenced `Depends-on:` footers in its
remediation rather than also covering `Depends on:` parent CL entries in the
commit body. Second, in `checks/commit_message_format.sh`, `fitting_tags`
checked whether any recurring tag in `non_leaf_tags` fit within 65 characters
instead of checking whether the directory's primary subsystem tag
(`non_leaf_tags[0]`) fit within 65 characters, causing shorter incidental or
secondary tags from cross-tree commits to falsely trigger
`commit_subject_missing_tag` on valid `[bazel_migration] //<dir>` subjects whose
primary subsystem tag would exceed the 65-character limit.

## Why not a one-off fix

Fixing only a single package's commit message or dependency stack leaves all
other stacked migrations with `Depends on:` body entries and long package paths
vulnerable to dropped parent stack remediations and false-positive
`commit_subject_missing_tag` warnings whenever shorter secondary tags appear in
`git log`.
