# Planter Installation Check

The `<planter>` below is the `planter` binary resolved with `"$(command -v planter || echo "$HOME/.local/bin/planter")"`, the same way `scripts/planter_task_add_run.sh` finds it. Use it instead of a bare `planter`, because `~/.local/bin` may not be on `PATH`.

## 1. Check the Planter Binary

1. Run `command -v planter || ls -l "$HOME/.local/bin/planter"` to find the `planter` binary.
   * If the binary is found, run `<planter> version` to verify that it starts.
   * If the binary is not found, or `<planter> version` fails, refer to [Install Planter](#2-install-planter) to install it, then repeat this check.

## 2. Install Planter

Planter is built from google3 (`//experimental/users/lindkvist/planter`) with `blaze`, so it must be built in a CitC client. Use the dedicated `planter-prebuilt` client for this, so the build never touches a client that holds the user's own pending changes.

1. Run `ls -d "/google/src/cloud/$USER/planter-prebuilt/google3"` to check whether the `planter-prebuilt` CitC client exists.
   * If it does not exist, run `g4 citc planter-prebuilt` to create it.
   * If it exists, run `cd "/google/src/cloud/$USER/planter-prebuilt/google3" && g4 sync` to pick up the latest Planter.
   * If `g4 citc` or `g4 sync` fails (e.g. sync conflicts), fail the Pre-Migration Checks step with the reason: "Failed to prepare the planter-prebuilt CitC client".

2. Run `cd "/google/src/cloud/$USER/planter-prebuilt/google3" && bash experimental/users/lindkvist/planter/scripts/publish_prebuilt.sh` to build Planter and install `planter` and `planter-rev` into `~/.local/bin`.
   * The `blaze build` can take 10 minutes or more. Allow a long timeout or run the command in the background and wait for it to finish; do not treat a slow build as a failure.
   * If the script fails, fail the Pre-Migration Checks step with the reason: "Failed to install Planter".

3. Run `"$HOME/.local/bin/planter" version` to verify the installation.
   * If it fails, fail the Pre-Migration Checks step with the reason: "Failed to install Planter".
   * If `command -v planter` still finds nothing, `~/.local/bin` is not on `PATH`. This is fine for the migration: `scripts/planter_task_add_run.sh` falls back to `~/.local/bin/planter`.

## 3. Check the Shared Machinery

1. Run `ls "$HOME/.planter/beads/build/beads/planter/machinery/prompts/coder.md"` to check whether the Planter shared machinery is set up.
   * If the file does not exist, run `<planter> init-project` from the root of the Fuchsia checkout to set it up.
   * If `<planter> init-project` fails, fail the Pre-Migration Checks step with the reason: "Failed to initialize the Planter shared machinery".

## 4. Check the Jetski Environment

Planter drives its coding and reviewer agents through the Jetski (Antigravity) language server. It reads the server address from `ANTIGRAVITY_LS_ADDRESS` (default `localhost:5387`) and the CSRF token from `ANTIGRAVITY_CSRF_TOKEN`, which are set in Jetski terminals.

1. Run `[[ -n "${ANTIGRAVITY_CSRF_TOKEN:-}" ]]` to check that the command runs in a Jetski terminal.
   * If it fails, fail the Pre-Migration Checks step with the reason: "Planter must run from a Jetski terminal".
