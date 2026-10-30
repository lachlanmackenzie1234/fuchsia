# Rust test lint config source fix scope and commit message alignment

- **Learned from:** inner loop friction on CL If360d6314b1adc3c4597bbbdcaea551770e5b024, patchset 6 (task `src-sys-run-test-suite`)
- **Date:** 2026-10-28
- **Changed:** `checks/build_only_scope.sh`, `checks/commit_message_format.sh`, `checks/lint_config_parity.sh`, `checks/manifest.json`

## Root cause

When a migrated Rust test target preserves a strict lint_config (such as
clippy_warn_all) from BUILD.gn instead of disabling clippy on tests, fixing
pre-existing test-only lints (such as removing redundant .clone() calls or
unused test imports) in source files is required to keep clippy enabled. While
build_only_scope and lint_config_parity had partial exemptions for explicit
rustc_test/host_rustc_test targets with lint_config, build_only_scope ignored
pure lint-removal hunks (such as deleted unused imports or standalone .clone()
lines), missed with_unit_tests/with_host_unit_tests targets, and emitted an
inaccurate unbuilt-package bitrot message, lint_config_parity skipped
lint-suppression attribute checks on those files, and commit_message_format
rejected natural commit body explanations of clippy/lint fixes because
bitrot_reason_re only matched unbuilt-package compile bitrot keywords.

## Why not a one-off fix

Allowing or special-casing source edits for a single crate directory would leave
future GN-to-Bazel migrations of Rust crates that preserve strict lint_config
settings on unit tests vulnerable to false-positive scope or lint errors on pure
removal hunks, unflagged #[allow]/#[expect] suppressions, and
commit_message_format rejections when documenting test clippy fixes.
