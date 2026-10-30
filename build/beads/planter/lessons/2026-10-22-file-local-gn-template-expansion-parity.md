# File local gn template expansion parity

- **Learned from:** inner loop friction on CL Ie75d2766b1ef0f3ddfd1370700350aadcd3dff0f, patchset 14 (task `starnix-net-matchers-ext`)
- **Date:** 2026-10-22
- **Changed:** `checks/gn_attr_parity.sh`, `checks/manifest.json`

## Root cause

When a pre-migration BUILD.gn defines a file-local template("<tmpl>") wrapping
an inner rule(<target_name>) and invokes that template to produce one or more
targets, diffengine parses the inner rule(target_name) inside the template body
as a target with the package's default name and pairs it with the primary
BUILD.bazel target while leaving the template invocation unpaired
(unmatched_target), and compares each target against only half of the unexpanded
template (either the template body without invocation deps or the invocation
without template-body deps and rule mapping). gn_target_type_parity.sh already
expanded file-local templates, but gn_attr_parity.sh did not record INFO
coverage for local template expansions or vendor crate matches in
template/feature_set extra deps.

## Why not a one-off fix

Hardcoding a target-specific exception would only silence findings for one
library directory, whereas file-local GN templates parameterizing shared
sources, deps, and crate features across multiple library variants (such as
standard and fdomain or feature-flagged builds) occur across the Fuchsia tree
and must be handled generically by parsing template() definitions in
gn_attr_parity.sh.
