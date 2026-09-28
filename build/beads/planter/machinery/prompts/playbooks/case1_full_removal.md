# Playbook: Case 1 Full BUILD.gn Removal

Use only when nothing GN-only has to stay in the package.
- Every GN target gets a 1:1 Bazel equivalent (same deps, testonly, configs); delete BUILD.gn
  only once no GN file references the package's targets.
- Register the migrated targets in the Bazel build verification lists instead of
  `verify_bazel2gn`.
