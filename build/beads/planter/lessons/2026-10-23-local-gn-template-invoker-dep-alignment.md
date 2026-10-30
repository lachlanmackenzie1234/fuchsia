# Local gn template invoker dep alignment

- **Learned from:** inner loop friction on a local task (task `starnix-fxfs-platform`)
- **Date:** 2026-10-23
- **Changed:** `checks/gn_attr_parity.sh`, `checks/manifest.json`

## Root cause

When a file-local GN template body appended or assigned invoker.<dep_attr> (such
as deps += invoker.deps) and was expanded into explicit BUILD.bazel targets,
parse_gn_top_attrs only extracted quoted string literals from the inner rule
body while diffengine included the unquoted invoker.<dep_attr> token in
unmatched GN dependencies, causing gn_attr_parity's dep_alignment INFO coverage
message to omit invoker.<dep_attr> and leaving a false-positive review_diff
warning.

## Why not a one-off fix

Suppressing the warning for a single package or forcing redundant
@bazel2gn:path_overwrite annotations would either fail in bazel_minimality or
leave other packages that expand file-local GN templates with invoker.<dep_attr>
dependency concatenations exposed to the same false-positive dep_alignment
warning.
