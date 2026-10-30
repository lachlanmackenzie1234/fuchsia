# Bazel2gn test package visibility sync and rdeps

- **Learned from:** human review on CL I675c7334abab3dc5ec6183ac6ae11b3e3cfc8864, patchset 1 (task `src-performance-memory-batch-heapdump_vmo`)
- **Date:** 2026-10-28
- **Changed:** `checks/visibility_audit.sh`, `tools/find_rdeps.sh`

## Root cause

Both find_rdeps.sh and visibility_audit.sh initialized gn_only_targets and
gn_nonconvertible_targets with {'tests', 'benchmarks'} and removed
below_synced_targets only from non_gn_synced_targets and package_targets, not
from gn_only_targets or gn_nonconvertible_targets. When a Bazel test package
(such as fx_unittest_package) was itself named 'tests' and synced below the
BAZEL2GN SENTINEL in BUILD.gn without a separate pre-sentinel group('tests'), GN
callers referencing ':tests' were misclassified as GN-only references and
rejected from Bazel visibility as false_positive_rdep_visibility, forcing the
coder to annotate the visibility attribute with '# @bazel2gn:skip' so GN
visibility would remain unrestricted while Bazel visibility satisfied the audit.
Additionally, visibility_audit.sh did not reject attribute-level '#
@bazel2gn:skip' on visibility of bazel2gn-synced targets.

## Why not a one-off fix

Fixing only a single package's BUILD.bazel leaves find_rdeps.sh and
visibility_audit.sh blind to GN reverse dependencies of any bazel2gn-synced
target named 'tests' or 'benchmarks' across the repository, and allows future
migrations to work around GN/Bazel visibility mismatches by placing '#
@bazel2gn:skip' on the visibility attribute instead of letting bazel2gn
translate visibility.
