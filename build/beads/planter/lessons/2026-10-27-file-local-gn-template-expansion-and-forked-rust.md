# File local gn template expansion and forked rust crate diff parity

- **Learned from:** inner loop friction on CL If360d6314b1adc3c4597bbbdcaea551770e5b024, patchset 3 (task `src-sys-run-test-suite`)
- **Date:** 2026-10-27
- **Changed:** `checks/gn_attr_parity.sh`, `checks/manifest.json`, `tools/run_checks.sh`

## Root cause

gn_attr_parity only queried the on-disk planter executable rather than also
querying any running planter daemon in /proc/<pid>/exe, did not match 'Unit
tests of "<target>"' unit_test_parity findings on expanded file-local GN
template targets (where diffengine parses 'invoker.features' from the template
body), and did not recognize //third_party/rust_crates:<crate> to
//third_party/rust_crates/forks/<dir>:<target> mappings; additionally,
tools/run_checks.sh used an elif chain in downgrade_check_cfg_diff_findings
where an early broad dep_alignment/unit_test_parity branch skipped subsequent
downgrade checks when its inner condition was false.

## Why not a one-off fix

File-local GN templates (such as dual-compiled FDomain/FIDL library templates)
and forked third-party Rust crates under //third_party/rust_crates/forks/ appear
across many Fuchsia packages, so reconciling their diffengine findings
generically in gn_attr_parity and run_checks prevents false positives across all
future migrations.
