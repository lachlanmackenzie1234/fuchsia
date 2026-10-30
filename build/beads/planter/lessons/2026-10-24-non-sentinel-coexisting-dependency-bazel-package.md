# Non sentinel coexisting dependency bazel package

- **Learned from:** inner loop friction on a local task (task `starnix-fasttime`)
- **Date:** 2026-10-24
- **Changed:** `prompts/reviewers/target_parity.md`

## Root cause

Item 9 in prompts/reviewers/target_parity.md unconditionally instructed the
reviewer to require complete target migration and a bazel2gn sentinel for every
transitive dependency directory added by the change, causing the reviewer to
reject non-sentinel BUILD.bazel files in dependency packages whose BUILD.gn
relies on kernel-only conditionals, complete_static_lib, custom generator
templates, or CPU select conditions unsupported by bazel2gn.

## Why not a one-off fix

Exempting only a single dependency package path would fail whenever future
migrations depend on other dual-environment kernel/userspace libraries whose
BUILD.gn uses is_kernel conditionals, custom action templates, or
non-bazel2gn-convertible attributes and therefore must coexist with a
non-sentinel BUILD.bazel.
