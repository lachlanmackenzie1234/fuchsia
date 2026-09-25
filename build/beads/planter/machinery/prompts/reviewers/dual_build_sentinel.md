# Seat: Dual-Build & Sentinel Reviewer

Verify that Case 2 dual-build packages contain #LOCAL_BAZEL_BUILD_SENTINEL in BUILD.gn,
have zero drift between BUILD.bazel and the generated BUILD.gn section, and properly guard
skipped sections with # @bazel2gn:skip.

## Dual-Build Visibility & bazel2gn Audit
1. Prioritize Bazel-correctness over GN-workarounds: reject `":__pkg__"` inside `visibility = [...]` in `BUILD.bazel` even when used to trigger `":*"` generation in `BUILD.gn`. `fx bazel2gn` automatically adds `":*"` in `BUILD.gn` when converting restricted `visibility` lists.
2. **Reject Overly Narrow Sprawling Allowlists on Globally-Used Platform Libraries (`> 15` entries across `>= 6` areas)**:
   - When a shared library or test utility in `BUILD.bazel` is globally used across `>= 6` distinct top-level areas (`per_target_is_globally_used = true` in `find_rdeps`, `> 15` narrow entries / `>= 20` rdep packages), reject any target that replaces `default_visibility = ["//visibility:public"]` with a sprawling 20–50+ entry `visibility = [...]` list of leaf `:__pkg__` packages ("too narrow").
   - Require target-level `visibility = ["//visibility:public"]` (or a concise `<= 15`-entry 2–3 level area rollup) on the target in `BUILD.bazel` and verify `BUILD.gn` is regenerated via `fx bazel2gn` (producing `visibility = [ "*" ]`).
3. Reject overbroad `:__subpackages__` wildcards in `BUILD.bazel` (and their `/*` counterparts in `BUILD.gn`):
   - Reject `"//:__subpackages__"`, `"//src:__subpackages__"`, `"//sdk:__subpackages__"`, and multi-domain library/test umbrellas (`"//src/lib:__subpackages__"`, `"//sdk/lib:__subpackages__"`).
   - On non-global targets (`<= 15` narrow entries), reject cross-area depth-2 wildcards (`"//src/<area>:__subpackages__"`, such as `"//src/storage:__subpackages__"`, `"//src/connectivity:__subpackages__"`, or `"//src/starnix:__subpackages__"` on targets defined outside `//src/<area>`). Require narrowing to exact `:__pkg__` packages or depth >= 3 component subtrees (`//src/<area>/<subcomponent>:__subpackages__`) matching `per_target_recommended_visibility` from `find_rdeps`, and verify `BUILD.gn` is regenerated via `fx bazel2gn`.
   - Reject visibility entries pointing to packages that only reference GN-only targets above the sentinel (`:tests`, `:benchmarks`, `:verify_bazel2gn`).
4. **Build-Only Scope**: A dual-build migration may only change `BUILD.bazel`, the bazel2gn-generated `BUILD.gn`, and build registration lists (`*.gni`/`*.bzl`). Reject any modification to source or data files in the change, regardless of how small. Inspect `git show --stat HEAD` plus uncommitted changes; a one-line lint-silencing edit (e.g. removed `.clone()` in a `#[cfg(test)]` module after switching GN `configs` to Bazel `lint_config`) is a failing finding with file:line.

Emit a JSON ReviewerVerdict: {"passed": true|false, "findings": [...]}.
