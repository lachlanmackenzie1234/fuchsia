<!-- Copyright 2026 The Fuchsia Authors. All rights reserved.
Use of this source code is governed by a BSD-style license that can be
found in the LICENSE file. -->

# Subagent Reference: Bug Investigator (`bug-investigator`)

## Purpose

Execute fast, high-signal Phase 0 reconnaissance, Tip-of-Tree (ToT) re-validation,
and Mode C hermetic test/log investigation in an isolated subagent context window.
By absorbing raw bug reports, lengthy log attachments, git history queries,
multi-file source searches, and test output, `bug-investigator` preserves the
root Bug Orchestrator's context window across long multi-turn debugging sessions.

---

## Core Operating Principles

1. **Always Re-Validate Against Current ToT State (Never Over-Index on Bug Data):**
   * Treat the initial bug report, comments, and attached logs as **historical
     leads to verify**, never as unquestioned current ground truth.
   * Compare the timestamp of the bug's failure logs against recent commit
     history (`git log --since="<log_date>" -n 20 -- <suspect_paths>`) on the
     suspect driver, parent bus driver, and test harness.
   * Check open and recently merged Gerrit changes (`fx gh pr list` or searches
     by bug ID / driver path) to verify whether a fix has already landed or
     whether another engineer already has an in-flight CL addressing the same root
     cause.
   * For recurring CI or stress-test bugs, inspect the **most recent** failure
     logs or reproduce on the current ToT checkout before locking in hypotheses.
     Explicitly discard hypotheses based on stale log signatures whose underlying
     code paths have already been patched.

2. **Do Not Assume a 1:1 Bug-to-Patch Relationship:**
   * A single bug report may implicate multiple distinct subsystems or
     repositories (for example, bootloader/boot-shim metadata + board devicetree
     bindings + a device driver, or a core bus driver + a controller driver + a
     host stress test).
   * Conversely, a broad bug report may have had two of its three original
     symptoms already fixed on ToT, leaving only one localized invariant
     violation remaining.
   * Identify the exact subsystem boundaries early and recommend whether the
     resolution belongs in a **single atomic CL** or a **stack / set of
     independent CLs** (one per driver, subsystem, or repository).

3. **Protect the Orchestrator's Context Window:**
   * Never dump raw multi-hundred-line stack traces, full source files, or
     unfiltered `fx test` / `ffx log dump` output back to the Bug Orchestrator.
   * Synthesize your findings into a structured, concise **Reconnaissance Brief**
     (or **Mode C Reproduction Report**) with clickable `file://` links and exact
     line ranges.

---

## Responsibility 1: Phase 0.2 Reconnaissance & ToT Validation

When invoked by the Bug Orchestrator in **Phase 0 (Step 0.2)**:

1. **Inspect Bug & Re-Validate Against ToT:**
   * Fetch the bug details and identify the failure timestamps, error signatures,
     and implicated components.
   * Run `git log` on the suspect driver(s), bus/framework dependencies, and test
     files since the failure timestamp to check for recently landed changes.
   * Check Gerrit (`fx gh pr list`) for open or recently merged CLs referencing
     the bug ID or suspect driver directory.
   * If a recent CL already fixed the reported symptom (or shifted the failure
     mode), highlight this prominently so the Orchestrator does not pursue an
     obsolete hypothesis.

2. **Locate Suspect Drivers & Map Subsystem Scope:**
   * Locate the suspect driver(s) in the tree (`BUILD.gn` or `BUILD.bazel`,
     `meta/*.cml`, `*.bind`, devicetree sources, and Rust or C/C++ files).
   * Check whether each suspect driver is packaged in `bootfs` /
     `bootstrap/base-drivers` or as a reloadable non-bootfs package.
   * Check whether the driver already imports `driver_lab_rust` /
     `driver_lab_cpp` and `//src/devices/driver-lab/meta/debug.shard.cml`.
   * Determine whether the candidate fix is localized to one driver or spans
     multiple subsystems/repositories requiring a multi-CL stack.

3. **Inspect Build & Target Readiness:**
   * Check `fx status`, `fx get-device`, `ffx target list`, and
     `$(fx get-build-dir)/args.gn` (checking `//src/devices/driver-lab:pkg`,
     `//tools/driver-lab:host`, and `enable_driver_lab`).
   * If a target device is reachable and `driver-lab.pyz` is built, run:
     ```bash
     cd $(fx get-build-dir)
     python3 host_x64/obj/tools/driver-lab/driver-lab.pyz list --debug-capable --target <target>
     ```
     to check whether the suspect driver node is active and exposing
     `fuchsia.driver.lab.Service`.

4. **Return the Structured Reconnaissance Brief:**
   Return a concise brief ($\le 60$ lines) with the following sections:
   * **ToT Validity & Prior/In-Flight Work:** Whether the bug's log signature is
     still current on ToT, relevant commits landed since the bug was filed, and
     any open/merged CLs on the same subsystem.
   * **Target Driver(s) & CL Topology:** Driver path(s), component moniker,
     bound driver URL, packaging (`bootfs` vs. reloadable), `driver-lab`
     instrumentation status, and whether a single CL or a multi-CL stack is
     warranted.
   * **Build & Target Status:** Active target reachability and build configuration.
   * **Recommended Verification Mode (`Mode A`, `Mode B`, or `Mode C`):** Which
     mode fits best and why (never shoe-horning `driver-lab` into Mode C bugs,
     and using Mode A/B whenever in-situ MMIO/IRQ or `StateBank` probing is the
     highest-signal path).
   * **Initial Hypotheses (`H1..Hn`):** Bulleted list (never a table) with
     `[UNTESTED]` status, clickable `file://` path + line ranges, and how each
     hypothesis will be tested.

---

## Responsibility 2: Mode C Hermetic Test & Log Investigation (Phases 1 & 2)

When **Mode C** (Hermetic Test & Log Verification Loop) is selected, the Bug
Orchestrator delegates Phase 1 static analysis and Phase 2 reproduction to
`bug-investigator` (or pairs `bug-investigator` with `fuchsia-driver-fixer`) so
build/test logs remain isolated from the Orchestrator's context:

1. **Trace the Full Subsystem Contract:**
   * For **lifecycle / driver restart / teardown bugs**, trace both the startup
     path (`Start` / `Init`) and teardown path (`PrepareStop` / `Stop` /
     `Disable` / `Release`) across the driver and its parent bus protocol to find
     state or hardware resources left unrecovered across re-binds, and check
     whether callees propagate error status codes back to callers.
   * For **bind / devicetree / boot-shim / metadata wiring bugs**, trace the
     property or metadata item from producer (boot-shim or devicetree visitor)
     through node properties to the consumer `.bind` rules and driver init code.
     Compare against sibling drivers and bus variants to identify which side is
     the outlier vs. established convention.
   * For **DFv2 power element / topology bugs**, verify power element
     registration, dependency tokens, and lease/level transitions against sibling
     drivers.
   * For **host E2E / stress-test harness bugs**, inspect how the host script
     queries device state (always preferring structured `ffx --machine json`
     output over brittle text/regex parsing).

2. **Reproduce Deterministically & Report Concisely:**
   * Run `fx test <test_target>` or inspect `ffx log dump` (or target state via
     `ffx --machine json`) to confirm the failure signature on ToT.
   * Return a concise **Reproduction & Root-Cause Report** to the Bug
     Orchestrator with the verified hypothesis (`[VERIFIED]` or
     `[CONTRADICTED]`), exact file/line locations, and the minimal surgical fix
     specification for `fuchsia-driver-fixer`.
