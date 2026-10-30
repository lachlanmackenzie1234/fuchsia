# Dual build testonly component migration and visibility

- **Learned from:** human review on CL I4fda514490455dbe58280c229090f8d9f704d911, patchset 2 (task `src-security-tests-pkg-test-batch`)
- **Date:** 2026-10-28
- **Changed:** `prompts/coder.md`

## Root cause

The coder prompt did not state that a GN fuchsia_test_component without
test_type (such as a mock or helper child component included in deps of a test
package) is equivalent to fuchsia_component(testonly = true) and should be
migrated to fx_component(testonly = True) with visibility from find_rdeps and
without # @bazel2gn:skip so bazel2gn generates the GN component target below the
sentinel.

## Why not a one-off fix

Fixing only the helper components in the single target package batch would leave
the coding agent prone to misclassifying other testonly mock/helper
fuchsia_test_component targets across the tree as unconvertible
fx_test_component or # @bazel2gn:skip targets lacking visibility.
