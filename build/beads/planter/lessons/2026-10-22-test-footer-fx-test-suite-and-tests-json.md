# Test footer fx test suite and tests json validation

- **Learned from:** inner loop friction on CL I6b32c2a580c2d73862b8e84a411b275519ecf8cb, patchset 13 (task `starnix-net-sockets-ext`)
- **Date:** 2026-10-22
- **Changed:** `checks/build_verification.sh`, `checks/commit_message_format.sh`, `checks/manifest.json`

## Root cause

First, build_verification.sh validated `Test: fx test //<pkg>:<target>` footers
using `any(t == lbl or t.startswith(pkg_prefix + ':') for t in
tests_json_targets)`, which falsely accepted any explicit `:<target>` (such as
`:bazel_tests` or `:tests`) whenever the same package had at least one real test
entry in `<build dir>/tests.json`, and also skipped footer verification when
`HEAD` matched `PLANTER_UPLOADED_COMMIT`. Second, commit_message_format.sh did
not reject `Test: fx test` footers that pass a GN `bazel_test_suite()`, GN
`group()`, or Bazel `test_suite()` target (or a target missing from `<build
dir>/tests.json`), and its `test_footer_not_ancestor_suite` remediation
suggested `fx test --host //<pkg>:tests` even though `fx test` only accepts
individual test targets in `tests.json` or package directory prefixes.

## Why not a one-off fix

Every dual-build or Bazel test migration defines a GN
`bazel_test_suite("bazel_tests")` wired into `group("tests")` alongside
individual test packages. Fixing only one package's commit message leaves both
`commit_message_format.sh` and `build_verification.sh` blind to `Test: fx test
//<pkg>:bazel_tests` or `//<pkg>:tests` across all packages that have at least
one real test in `tests.json`.
