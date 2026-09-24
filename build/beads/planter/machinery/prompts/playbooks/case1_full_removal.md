# Playbook: Case 1 Full BUILD.gn Removal

- Ensure every target in the former BUILD.gn has a 1:1 semantic equivalent in BUILD.bazel.
- Preserve target visibility, testonly flags, deps, and configs.
- Register migrated targets in global Bazel build verification lists.
- **Visibility Scoping**:
  - Do not set `default_visibility` in `package(...)`.
  - Scope target `visibility` using only true reverse dependencies (`deps`, `public_deps`, `test_deps`, `proc_macro_deps`, `actual`). Never add packages that only reference this target inside their own `visibility = [...]` lists or `.gni` visibility allowlists.
  - Never include redundant `":__pkg__"` entries in `visibility = [...]` lists, and never use `"//:__subpackages__"` or enumerate 5+ top-level `//<dir>:__subpackages__` entries. Use target-level `visibility = ["//visibility:public"]` only when the target is genuinely a tree-wide or SDK-published public API.
- **Build-Only Scope**:
  - Only `BUILD.gn` (deletion), `BUILD.bazel`, and build registration lists (`*.gni` / `*.bzl`) may change. Never edit source files (`.rs`, `.cc`, `.h`, `.py`, `.fidl`, `.cml`, etc.), even to silence a new lint or warning.
  - If the Bazel target fails lint/compile where GN passed, reproduce the GN `configs` in Bazel (`lint_config`, `rustc_flags`, `copts`, `features`) instead of touching sources; if impossible, stop and report.
  - GN `configs += [ <lint config> ]` appends to default lints; Bazel `lint_config` replaces the macro default. Expect lint-set drift (including in `with_unit_tests` test code) and never "fix" it in sources.
  - If Bazel clippy fails only on `with_unit_tests` test code, follow coder rule 6 (explicit `rustc_test` with the area's test lint config).
  - Before finishing, confirm `git status --porcelain`, `git diff --name-only HEAD`, and `git show --stat HEAD` list only build-definition files.
