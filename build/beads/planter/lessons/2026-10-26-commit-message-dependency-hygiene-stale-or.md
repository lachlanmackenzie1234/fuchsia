# Commit message dependency hygiene: stale or unresolved Depends-on references in migration CL descriptions

- **Learned from:** human review on CL I733d05125173d968298e788a9a4ec995ea38f014, patchset 18 (task `starnix-core`)
- **Date:** 2026-10-26
- **Changed:** `checks/commit_depends_on.sh`

## Root cause

commit_message_format validates subject, Bazel-Migration-Target, Test: and
footer layout, but nothing checked the 'Depends on:' block or Depends-on:
footers. So the coder's unresolved local task names (e.g. 'CL <task-name>') and
entries for changes that had already merged went through all rounds unflagged.

## Why not a one-off fix

Every stacked migration task writes Depends-on entries from local task names and
stops updating them once parents merge. Hand-editing one CL's message leaves the
same defect in every later migration commit, while a deterministic check catches
it on every task.
