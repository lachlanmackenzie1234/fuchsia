# Playbook: Case 2 Dual-Build (bazel2gn)

Use when the package keeps a BUILD.gn (it has GN-only constructs such as test or component
packages, or GN dependents that must keep working).
- BUILD.bazel is the source of truth; never hand-edit BUILD.gn below the sentinel. After every
  BUILD.bazel edit, run `fx bazel2gn -d <dir>`.
- Everything the coder prompt says about labels, edge kinds, lint parity and build arguments
  applies to the regenerated BUILD.gn: GN dependents in other packages build against it.
