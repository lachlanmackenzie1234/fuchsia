# Bazel action sandboxing and undeclared header dependency hygiene

- **Learned from:** human review on CL I2ea94ff7f1265f1cb0439b8d2c9c8b0043802890, patchset 4 (task `starnix-modules-hw-power`)
- **Date:** 2026-10-30
- **Changed:** `checks/bazel_minimality.sh`, `checks/gn_attr_parity.sh`

## Root cause

Because GN does not sandbox C/C++ compilation and places the checkout root on
the default include path, a GN source_set or library can #include headers from
another directory (such as a third-party subtree with a Bazel-only header
target) without declaring a GN dependency. When migrating or updating such a
target in Bazel, agents sometimes worked around missing-header errors in the
Bazel sandbox by adding `tags = ["no-sandbox"]` (with `# @bazel2gn:skip`)
instead of declaring the header library target in `implementation_deps` or
`deps`. Neither `bazel_minimality.sh` nor `gn_attr_parity.sh` rejected `tags =
["no-sandbox"]` on targets in `BUILD.bazel`, and `gn_attr_parity.sh` did not
record `dep_alignment` coverage when a Bazel C/C++ target added a header
dependency for an include path that GN resolved directly from the source tree.

## Why not a one-off fix

Removing `tags = ["no-sandbox"]` only in a single package leaves the underlying
anti-pattern unguarded whenever future migrations encounter C/C++ targets whose
sources include cross-package headers without an explicit GN dependency.
Enforcing a deterministic error against `tags = ["no-sandbox"]` in
`bazel_minimality.sh` with automatic header-target remediation discovery, paired
with `dep_alignment` coverage in `gn_attr_parity.sh`, prevents sandbox-disabling
workarounds across all future migrations.
