# Playbook: Case 1 Full BUILD.gn Removal

Use when nothing GN-only has to stay in the package (e.g. a pure `fuchsia_package` /
`fuchsia_package_with_single_component` directory with no GN test package that must stay in GN
and no GN callers).
- Convert every GN target to its Bazel equivalent (`fx_package`, `fx_component`,
  `fx_component_manifest`, `fx_packaged_binary`, `resource`, or library/binary rules) and delete
  `BUILD.gn` (`git rm <dir>/BUILD.gn`).
- A package with Rust device unit tests cannot be fully removed: its GN test package wraps the
  bazel2gn-generated `:<name>_test`, so use Case 2. C++ device tests and host tests GN does not
  otherwise build migrate per "Migrating Tests" in the coder prompt; they still need a GN
  `bazel_test_suite` listed by `group("tests")` to reach `tests.json`, so such a package keeps a
  BUILD.gn holding only those two targets (no sentinel, no `verify_bazel2gn`).
- For assembly packages bridged via `//bundles/assembly/bazel_inputs/<dir>`, switch
  `//bundles/assembly/BUILD.bazel` to `//<dir>:<pkg>`, remove the entry from
  `//bundles/assembly/bazel_inputs/BUILD.gn`, and delete
  `//bundles/assembly/bazel_inputs/<dir>/BUILD.*`.
