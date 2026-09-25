# Playbook: Case 1 Full BUILD.gn Removal

- Ensure every target in the former BUILD.gn has a 1:1 semantic equivalent in BUILD.bazel.
- Preserve target visibility, testonly flags, deps, and configs.
- Register migrated targets in global Bazel build verification lists.
- **Visibility Scoping**:
  - Do not set `default_visibility` in `package(...)`.
  - Scope target `visibility` per-target using only true reverse dependencies (`deps`, `public_deps`, `test_deps`, `proc_macro_deps`, `actual`) via `find_rdeps`. Exclude packages that only reference this package inside `visibility = [...]`, `visibility.gni`, or GN-only aggregator/verification targets (`:verify_bazel2gn`, `:tests`, `:benchmarks`).
  - **Globally-Used Platform Libraries & Test Utilities (`per_target_is_globally_used = true`)**: When a target is depended upon across `>= 6` distinct top-level areas and would otherwise require `> 15` narrow visibility entries (`>= 20` rdep packages), do NOT paste a 20–50+ line allowlist of leaf `:__pkg__` entries ("too narrow"). Set target-level `visibility = ["//visibility:public"]` (or a concise `<= 15`-entry 2–3 level area rollup) as recommended by `find_rdeps`.
  - Never include redundant `":__pkg__"` entries in `visibility = [...]` lists, and never use `"//:__subpackages__"`, top-level `"//src:__subpackages__"` / `"//sdk:__subpackages__"`, or multi-domain umbrellas (`"//src/lib:__subpackages__"`, `"//sdk/lib:__subpackages__"`).
  - For non-global targets (`<= 15` narrow entries), never roll up callers in an external area into a depth-2 `"//src/<area>:__subpackages__"` wildcard (e.g. `"//src/storage:__subpackages__"`, `"//src/connectivity:__subpackages__"`, `"//src/starnix:__subpackages__"` on targets outside that area). Always list exact `:__pkg__` entries or depth >= 3 component subtrees (`//src/<area>/<subcomponent>:__subpackages__`) matching the Lowest Common Ancestor of actual callers.
- **Build-Only Scope**:
  - Only `BUILD.gn` (deletion), `BUILD.bazel`, and build registration lists (`*.gni` / `*.bzl`) may change. Never edit source files (`.rs`, `.cc`, `.h`, `.py`, `.fidl`, `.cml`, etc.), even to silence a new lint or warning.
  - If the Bazel target fails lint/compile where GN passed, reproduce the GN `configs` in Bazel (`lint_config`, `rustc_flags`, `copts`, `features`) instead of touching sources; if impossible, stop and report.
  - GN `configs += [ <lint config> ]` appends to default lints; Bazel `lint_config` replaces the macro default. Expect lint-set drift (including in `with_unit_tests` test code) and never "fix" it in sources.
  - If Bazel clippy fails only on `with_unit_tests` test code, follow coder rule 6 (explicit `rustc_test` with the area's test lint config).
  - Before finishing, confirm `git status --porcelain`, `git diff --name-only HEAD`, and `git show --stat HEAD` list only build-definition files.
