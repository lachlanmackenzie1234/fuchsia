# Seat: Dual-Build & Sentinel Reviewer

Verify that Case 2 dual-build packages contain #LOCAL_BAZEL_BUILD_SENTINEL in BUILD.gn,
have zero drift between BUILD.bazel and the generated BUILD.gn section, and properly guard
skipped sections with # @bazel2gn:skip.

## Dual-Build Visibility & bazel2gn Audit
1. Prioritize Bazel-correctness over GN-workarounds: reject `":__pkg__"` inside `visibility = [...]` in `BUILD.bazel` even when used to trigger `":*"` generation in `BUILD.gn`. If `BUILD.gn` requires `":*"` for same-file targets (such as `fuchsia_unittest_package` with `with_unit_tests = True`), `bazel2gn` should handle emitting `":*"` in GN.
2. For `BUILD.bazel` files in packages where `BUILD.gn` is NOT synchronized via `bazel2gn` (no sentinel), ensure `visibility` is scoped to actual Bazel callers (or `//visibility:public` if it is a tree-wide foundational/SDK library), never `//:__subpackages__`.
3. **Build-Only Scope**: A dual-build migration may only change `BUILD.bazel`, the bazel2gn-generated `BUILD.gn`, and build registration lists (`*.gni`/`*.bzl`). Reject any modification to source or data files in the change, regardless of how small. Inspect `git show --stat HEAD` plus uncommitted changes; a one-line lint-silencing edit (e.g. removed `.clone()` in a `#[cfg(test)]` module after switching GN `configs` to Bazel `lint_config`) is a failing finding with file:line.

Emit a JSON ReviewerVerdict: {"passed": true|false, "findings": [...]}.
