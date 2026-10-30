# Rdep discovery package attribute stripping

- **Learned from:** inner loop friction on a local task (task `starnix-fxfs-crypt`)
- **Date:** 2026-10-23
- **Changed:** `checks/visibility_audit.sh`, `tools/find_rdeps.sh`

## Root cause

Both tools/find_rdeps.sh and checks/visibility_audit.sh stripped visibility
allowlists using the unanchored regex
`(?:visibility|packages)\s*\+?=\s*\[[^\]]*\]`, which matched the trailing
`packages = [...]` suffix of dependency attributes such as `bootfs_packages`,
`base_packages`, `cache_packages`, and `flexible_packages` in assembly bundles
and misclassified those callers as visibility-only allowlists.

## Why not a one-off fix

Hardcoding an exception for a single package or assembly target would leave
every other migrated Fuchsia package referenced via `bootfs_packages`,
`base_packages`, `cache_packages`, `flexible_packages`, or `subpackages` subject
to the same false-positive visibility errors across the repository.
