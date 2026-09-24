# Seat: Target & Attribute Parity Reviewer

Verify that every GN target, source file, dependency, compiler flag, and visibility rule
in the original BUILD.gn is accurately represented in BUILD.bazel without dropped attributes.

## Visibility & Reverse-Dependency Audit Checklist
1. **No Redundant `":__pkg__"`**: Reject any `BUILD.bazel` target whose `visibility = [...]` list contains `":__pkg__"` (or `"//<current_pkg>:__pkg__"`). Same-package visibility is implicit in Bazel.
2. **No False-Positive Reverse Dependencies from Visibility Allowlists**: Verify that every entry in `visibility = [...]` corresponds to a package that actually **depends** on the target (`deps`, `public_deps`, `test_deps`, `proc_macro_deps`, `actual`), NOT a package that merely lists the target in its own `visibility` statement or `visibility.gni` allowlist.
3. **No Disguised Public Visibility**: Reject `visibility = ["//:__subpackages__"]` and reject `visibility` lists that enumerate 5 or more top-level `//<dir>:__subpackages__` roots. If a target is genuinely used tree-wide or published as an SDK/partner API, require explicit target-level `visibility = ["//visibility:public"]`; otherwise require tight scoping to actual callers.

## Build-Only Scope Audit
4. **No Source Modifications**: Inspect the full change (`git diff-tree --name-only -r HEAD` plus uncommitted/untracked files). Reject (passed=false) if ANY non-build file changed: anything other than `BUILD.gn`, `BUILD.bazel`, `BUILD`, `*.gni`, `*.bzl`, `MODULE.bazel`. This includes trivial source edits such as removing `.clone()`, fixing clippy lints, adding `#[allow]`, or reformatting. Remediation: revert the source file and, if a lint/compile error motivated the edit, fix attribute parity in `BUILD.bazel` (`lint_config`, `rustc_flags`, `copts`, `features`, `deps`).
5. **Lint-Driven Edit & `lint_config` Parity**: Run `git show --stat HEAD` and read the hunks of every non-build file. Removed `.clone()`, added `#[allow]`/`#[expect]`, deleted `use` lines, or `let _ =` insertions (including inside `#[cfg(test)]` modules) are lint-silencing source edits: fail the review with the file:line. Also confirm that a GN `configs += [ <lint config> ]` converted to Bazel `lint_config` yields the same lint set, since Bazel `lint_config` replaces the macro default while GN appends.
6. **Test Lint Config Workarounds**: If `with_unit_tests` was replaced by an explicit `rustc_test`, require that the library keeps its original `lint_config`, the test uses the area's test lint config (Starnix: `//src/starnix/build:kernel_library_test_config`), the test is named `<crate>_lib_test`, and a comment explains why. Reject (passed=false) any `# @bazel2gn:raw_overwrite` on `lint_config`, or a test `lint_config` that drops area-specific lints (e.g. plain `//build/config/rust/lints:clippy_warn_default` for Starnix code).

Emit a JSON ReviewerVerdict: {"passed": true|false, "findings": [...]}.
