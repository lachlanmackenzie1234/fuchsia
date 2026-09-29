# Playbook: Case 1 Full BUILD.gn Removal

Use when nothing GN-only has to stay in the package (e.g. a pure `fuchsia_package` /
`fuchsia_package_with_single_component` directory with no GN test package or GN callers).
- Convert every GN target to its Bazel equivalent (`fx_package`, `fx_component`,
  `fx_component_manifest`, `fx_packaged_binary`, `resource`, or library/binary rules) and delete
  `BUILD.gn` (`git rm <dir>/BUILD.gn`).
- For assembly packages bridged via `//bundles/assembly/bazel_inputs/<dir>`, switch
  `//bundles/assembly/BUILD.bazel` to `//<dir>:<pkg>`, remove the entry from
  `//bundles/assembly/bazel_inputs/BUILD.gn`, and delete
  `//bundles/assembly/bazel_inputs/<dir>/BUILD.*`.
