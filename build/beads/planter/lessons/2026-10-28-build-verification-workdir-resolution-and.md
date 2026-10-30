# Build verification workdir resolution and unmounted checkout handling

- **Learned from:** inner loop friction on a local task (task `src-performance-memory-batch`)
- **Date:** 2026-10-28
- **Changed:** `checks/build_verification.sh`, `tools/run_checks.sh`

## Root cause

When PLANTER_WORKDIR is empty or unset (or when check scripts run from the
machinery pin directory or a package subdirectory), build_verification.sh
defaulted WORKDIR to '.' without resolving the enclosing git toplevel or
checking whether a Fuchsia checkout was actually mounted, and unconditionally
emitted a blocking ERROR finding (build_setup: Cannot find fx) before checking
PLANTER_SKIP_BUILD or whether workdir was inside PLANTER_PROJECT_DIR, whereas
all other checks (including tests_json_parity.sh) gracefully skipped with INFO
when no checkout or fx existed in workdir.

## Why not a one-off fix

Hardcoding or bypassing build_setup for a single target directory would leave
every future batch or pin-directory check execution failing with a blocking
build_setup ERROR whenever PLANTER_WORKDIR is unconfigured, points to the
machinery directory, or runs from a subdirectory of the checkout.
