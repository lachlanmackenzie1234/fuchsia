#!/usr/bin/env bash
# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

set -uo pipefail

# Deterministic check that actually builds the migration. Static checks and the
# review panel only read BUILD files; this is what proves the change compiles.
#
# Steps (each failing step becomes one ERROR finding with the relevant log lines):
#   1. gn_build:               `fx build` (incremental, the whole configured
#                              product), so the migrated GN targets and the GN
#                              dependents in the product graph are rebuilt.
#   2. gn_build_dependents:    `fx build <labels>` for the GN targets the product
#                              graph may leave out: the changed packages'
#                              `:tests` groups and test/binary targets, then the
#                              GN targets of other packages that depend on a
#                              changed package (tests and binaries first, then
#                              libraries), limited to labels of the GN graph
#                              (<build dir>/ninja_outputs.json). Catches GN-only
#                              link failures (e.g. undefined `operator new` in a
#                              GN test after a dependency label or edge changed)
#                              that Bazel builds and the default `fx build` miss.
#                              Skipped when gn_build fails.
#   3. bazel2gn_verifications: `fx build --host //build:bazel2gn_verifications`,
#                              which fails if a BUILD.gn has drifted from what
#                              `fx bazel2gn` generates from BUILD.bazel.
#   4. bazel_build_fuchsia:    `fx bazel build --config=fuchsia_platform //<dir>:all`
#                              for every changed directory with a BUILD.bazel
#                              (host-only targets are skipped as incompatible).
#   5. bazel_build_host:       `fx bazel build --config=host //<dir>:all` for the
#                              directories whose BUILD.bazel declares host targets.
#
# Directories: $PLANTER_TARGET_DIRS plus every directory whose BUILD.bazel or
# BUILD.gn changed since $PLANTER_CHANGE_BASE (e.g. migrated dependencies).
# With several target directories planter runs checks once per directory; this
# check only builds on the run for the first one.
#
# Environment:
#   PLANTER_SKIP_BUILD=1          skip the builds (reports an INFO finding).
#   PLANTER_BUILD_DRY_RUN=1       run nothing; report the planned commands as
#                                 INFO findings.
#   PLANTER_BUILD_TIMEOUT=<secs>  per-step timeout (default 5400).
#   PLANTER_DEPENDENT_BUILD_LIMIT=<n>  max labels for gn_build_dependents
#                                 (default 40).

WORKDIR="${PLANTER_WORKDIR:-.}"
TARGET_DIR="${PLANTER_TARGET_DIR:-}"
TARGET_DIRS="${PLANTER_TARGET_DIRS:-$TARGET_DIR}"

first_dir="${TARGET_DIRS%% *}"
if [[ -n "$TARGET_DIR" && -n "$first_dir" && "$TARGET_DIR" != "$first_dir" ]]; then
  echo "[]"
  exit 0
fi

python3 - "$WORKDIR" "$TARGET_DIRS" <<'PYEOF'
import json
import os
import re
import shutil
import subprocess
import sys

workdir = os.path.abspath(sys.argv[1])
target_dirs = [d.strip().strip("/") for d in sys.argv[2].split() if d.strip().strip("/")]
change_base = os.environ.get("PLANTER_CHANGE_BASE", "").strip() or "HEAD"
timeout = int(os.environ.get("PLANTER_BUILD_TIMEOUT", "5400"))
dry_run = os.environ.get("PLANTER_BUILD_DRY_RUN", "").strip() not in ("", "0")
dependent_limit = int(os.environ.get("PLANTER_DEPENDENT_BUILD_LIMIT", "40"))

if os.environ.get("PLANTER_SKIP_BUILD", "").strip() not in ("", "0"):
    print(json.dumps([{
        "source": "build_verification",
        "category": "build_skipped",
        "severity": "INFO",
        "message": "PLANTER_SKIP_BUILD is set; the migration was NOT built.",
    }], indent=2))
    sys.exit(0)


def git_lines(args):
    try:
        out = subprocess.check_output(["git", "-C", workdir] + args, stderr=subprocess.DEVNULL, text=True)
    except Exception:
        return []
    return [l.strip() for l in out.splitlines() if l.strip()]


dirs = list(target_dirs)
changed = git_lines(["diff", "--name-only", change_base]) + git_lines(["ls-files", "--others", "--exclude-standard"])
for p in changed:
    if os.path.basename(p) in ("BUILD.bazel", "BUILD.gn"):
        d = os.path.dirname(p).strip("/")
        if d and d != "build/bazel" and d not in dirs:
            dirs.append(d)

fx = None
for rel in (".jiri_root/bin/fx", "scripts/fx"):
    cand = os.path.join(workdir, rel)
    if os.path.isfile(cand):
        fx = cand
        break
if fx is None:
    print(json.dumps([{
        "source": "build_verification",
        "category": "build_setup",
        "severity": "ERROR",
        "message": f"Cannot find fx in {workdir} (.jiri_root/bin/fx or scripts/fx); the migration could not be built.",
    }], indent=2))
    sys.exit(0)

bazel_dirs = [d for d in dirs if os.path.isfile(os.path.join(workdir, d, "BUILD.bazel"))]
host_markers = re.compile(r"HOST_CONSTRAINTS|HOST_OS_CONSTRAINTS|_host_tool\b|host_go_test|with_host_unit_tests")
host_dirs = []
for d in bazel_dirs:
    try:
        with open(os.path.join(workdir, d, "BUILD.bazel")) as f:
            if host_markers.search(f.read()):
                host_dirs.append(d)
    except OSError:
        pass

LINKED_TEMPLATES = {"executable", "test", "rustc_binary", "rustc_test", "loadable_module", "shared_library", "rustc_cdylib", "fuchsia_driver"}
LIBRARY_TEMPLATES = {"rustc_library", "rustc_macro", "rustc_staticlib", "static_library", "source_set"}
GN_TARGET = re.compile(r'\b([A-Za-z_]\w*)\(\s*"([^"$]+)"\s*\)\s*\{')
GN_DEP_LIST = re.compile(r"(?<![\w.])(?:public_deps|deps|non_rust_deps|test_deps|data_deps)\s*\+?=\s*\[([^\]]*)\]")
GN_DEP_VAR = re.compile(r"(?<![\w.])(?:public_deps|deps|non_rust_deps|test_deps|data_deps)\s*\+?=\s*([A-Za-z_][^\n]*)")
LIST_VAR = re.compile(r"(?<![\w.])([A-Za-z_]\w*)\s*\+?=\s*\[([^\]]*)\]")
UNIT_TESTS = re.compile(r"(?<![\w.])with_unit_tests\s*=\s*true")


def read(rel):
    try:
        with open(os.path.join(workdir, rel), encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return None


def gn_code(text):
    """Blanks out GN comments without moving offsets, so strings and brackets can be scanned."""
    out, i, n, in_str = [], 0, len(text), False
    while i < n:
        c = text[i]
        if in_str:
            if c == "\\":
                out.append(text[i : i + 2])
                i += 2
                continue
            in_str = c != '"'
        elif c == '"':
            in_str = True
        elif c == "#":
            j = text.find("\n", i)
            j = n if j < 0 else j
            out.append(" " * (j - i))
            i = j
            continue
        out.append(c)
        i += 1
    return "".join(out)


def closing(code, i):
    """Index of the `}` closing the `{` at code[i]."""
    depth, in_str = 0, False
    while i < len(code):
        c = code[i]
        if in_str:
            if c == "\\":
                i += 2
                continue
            in_str = c != '"'
        elif c == '"':
            in_str = True
        elif c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    return len(code)


def gn_blocks(text):
    """{name: [template, body]} for every `template("name") { ... }` block of a BUILD.gn."""
    code = gn_code(text)
    blocks = {}
    for m in GN_TARGET.finditer(code):
        if m.group(1) in ("template", "declare_args", "foreach", "forward_variables_from"):
            continue
        brace = m.end() - 1
        blocks.setdefault(m.group(2), [m.group(1), ""])[1] += code[brace + 1 : closing(code, brace)]
    return blocks


def label_dir(raw, pkg):
    """Package directory of a GN label written in package pkg; None for labels built from variables."""
    raw = raw.strip()
    if not raw or "$" in raw or raw.startswith("@"):
        return None
    path = re.sub(r"\(.*\)$", "", raw).split(":", 1)[0]
    if path.startswith("//"):
        return path[2:].strip("/")
    return os.path.normpath(os.path.join(pkg, path)).strip("/") if path else pkg


def rdep_build_files():
    """BUILD.gn files outside the changed directories that mention one of them."""
    patterns = [a for d in dirs for a in ("-e", f"//{d}")]
    rg = shutil.which("rg", path=os.environ.get("PATH", "") + os.pathsep + os.path.expanduser("~/.cargo/bin"))
    cmd = (
        [rg, "-l", "--fixed-strings", "-g", "BUILD.gn"] + patterns + ["."]
        if rg
        else ["git", "grep", "-l", "--fixed-strings"] + patterns + ["--", "BUILD.gn", "*/BUILD.gn"]
    )
    try:
        out = subprocess.run(cmd, cwd=workdir, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, timeout=600).stdout
    except (OSError, subprocess.SubprocessError):
        return []
    files = {os.path.normpath(l.strip()) for l in out.splitlines() if l.strip()}
    return sorted(f for f in files if os.path.dirname(f) not in dirs and not f.startswith("out/"))


def load_toolchains():
    try:
        with open(os.path.join(workdir, ".fx-build-dir")) as f:
            build_dir = f.read().strip()
        with open(os.path.join(workdir, build_dir, "ninja_outputs.json")) as f:
            known = json.load(f)
    except (OSError, ValueError) as e:
        return None, str(e)
    toolchains = {}
    for key in sorted(known):
        toolchains.setdefault(key.split("(", 1)[0], []).append(key)
    return toolchains, None


def dependent_labels():
    """(labels, notes) for gn_build_dependents: the changed packages' `:tests` groups and test/binary/library targets,
    then GN targets of other packages that depend on a changed package (tests and binaries, `<name>_test` of
    `with_unit_tests` Rust targets, then libraries), as keys of <build dir>/ninja_outputs.json."""
    toolchains, err = load_toolchains()
    if toolchains is None:
        return [], [f"Cannot read the GN target list (<build dir>/ninja_outputs.json): {err}. GN dependents were not built."]
    own, linked, unit_tests, libraries = [], [], [], []
    unconfigured_own_tests = []
    for d in dirs:
        blocks = gn_blocks(read(f"{d}/BUILD.gn") or "")
        if "tests" in blocks:
            t_lbl = f"//{d}:tests"
            own.append(t_lbl)
            if t_lbl not in toolchains:
                unconfigured_own_tests.append(t_lbl)
        for name, (tmpl, body) in sorted(blocks.items()):
            if tmpl in LINKED_TEMPLATES or tmpl in LIBRARY_TEMPLATES:
                own.append(f"//{d}:{name}")
            if tmpl.startswith("rustc_") and UNIT_TESTS.search(body):
                own.append(f"//{d}:{name}_test")
    for rel in rdep_build_files():
        pkg = os.path.dirname(rel)
        text = read(rel) or ""

        def names_changed(items):
            return bool({label_dir(s, pkg) for s in re.findall(r'"([^"]*)"', items)} & set(dirs))

        # List variables (e.g. `_core_deps = [ ... ]`) naming a changed package, used as `deps = _core_deps`.
        dep_vars = {
            m.group(1)
            for m in LIST_VAR.finditer(gn_code(text))
            if m.group(1) not in ("public_deps", "deps", "non_rust_deps", "test_deps", "data_deps", "visibility")
            and names_changed(m.group(2))
        }
        for name, (tmpl, body) in sorted(gn_blocks(text).items()):
            if not any(names_changed(m.group(1)) for m in GN_DEP_LIST.finditer(body)) and not any(
                re.search(r"(?<![\w.])(%s)\b" % "|".join(map(re.escape, dep_vars)), m.group(1))
                for m in GN_DEP_VAR.finditer(body)
                if dep_vars
            ):
                continue
            if tmpl in LINKED_TEMPLATES:
                linked.append(f"//{pkg}:{name}")
            if tmpl.startswith("rustc_") and UNIT_TESTS.search(body):
                unit_tests.append(f"//{pkg}:{name}_test")
            if tmpl in LIBRARY_TEMPLATES:
                libraries.append(f"//{pkg}:{name}")
    labels = []
    for label in dict.fromkeys(own + linked + unit_tests + libraries):
        keys = toolchains.get(label, [])
        key = label if label in keys else next((k for k in keys if "host" in k), keys[0] if keys else None)
        if key and key not in labels:
            labels.append(key)
    notes = []
    if unconfigured_own_tests:
        notes.append(
            f"Not in the configured GN graph (<build dir>/ninja_outputs.json): {' '.join(unconfigured_own_tests)}. "
            f"Do NOT run `fx build {' '.join(unconfigured_own_tests)}` or include unconfigured GN labels in "
            "`CoderReport.tests_run` or `Test:` commit footers (Planter's test_verifier will fail with `Unknown GN label`)."
        )
    if len(labels) > dependent_limit:
        notes.append(
            f"gn_build_dependents builds only the first {dependent_limit} of {len(labels)} labels "
            f"(PLANTER_DEPENDENT_BUILD_LIMIT); not built: {' '.join(labels[dependent_limit:])}"
        )
        labels = labels[:dependent_limit]
    return labels, notes


steps = [
    ("gn_build", [fx, "build"]),
    ("gn_build_dependents", None),  # Labels are computed once gn_build has regenerated the GN graph.
    ("bazel2gn_verifications", [fx, "build", "--host", "//build:bazel2gn_verifications"]),
]
if bazel_dirs:
    steps.append(("bazel_build_fuchsia", [fx, "bazel", "build", "--config=fuchsia_platform"] + [f"//{d}:all" for d in bazel_dirs]))
if host_dirs:
    steps.append(("bazel_build_host", [fx, "bazel", "build", "--config=host"] + [f"//{d}:all" for d in host_dirs]))

ERROR_LINE = re.compile(
    r"(^ERROR:|^FAILED:|\berror(\[E\d+\])?:|^error\b|: error\b|\bundefined reference\b|"
    r"no such (target|package)|is not visible|Unable to load|not declared|drift|differs|--- a/|\+\+\+ b/)",
    re.I,
)
LOCATION = re.compile(r"(?:-->\s*)?((?:\.\./\.\./)?[\w./+-]+\.(?:rs|cc|h|c|gn|gni|bazel|bzl)):(\d+)")
BUILD_FILE_LOCATION = re.compile(r"((?:\.\./\.\./)?[\w./+-]*(?:BUILD\.gn|BUILD\.bazel|\.gni|\.bzl)):(\d+)")


def summarize(output, location=LOCATION):
    lines = output.splitlines()
    picked = []
    for i, line in enumerate(lines):
        if ERROR_LINE.search(line):
            for j in range(max(0, i - 1), min(len(lines), i + 4)):
                if not picked or picked[-1] < j:
                    picked.append(j)
    text = "\n".join(lines[j] for j in picked) if picked else "\n".join(lines[-40:])
    if len(text) > 6000:
        text = text[:6000] + "\n... (truncated)"
    file, line = "", 0
    m = location.search(text)
    if m:
        file = m.group(1).replace("../../", "", 1)
        if file.startswith(workdir + "/"):
            file = file[len(workdir) + 1:]
        line = int(m.group(2))
    return text, file, line


REMEDIATION = {
    "gn_build": "Fix the BUILD.bazel/BUILD.gn so the GN build passes (renamed/removed labels that dependents still use, "
                "missing deps, attribute drift), then re-run `fx bazel2gn -d <dir>` and `fx build`. Never edit sources to make it pass.",
    "gn_build_dependents": "A GN target that the default `fx build` does not build failed: a test or binary of a changed "
                           "package, or a GN dependent in another package. Undefined symbols at link time (e.g. `operator "
                           "new`/`operator delete` from C++ code linked into a Rust test) or missing configs mean the "
                           "migration changed a GN dependency label or edge kind: a GN-only wrapper (e.g. a `*_static` "
                           "target carrying link settings) replaced by the target it wraps, or a `public_deps` edge turned "
                           "into `deps`. Restore the original labels and edge kinds in the migrated packages (see the "
                           "`gn_dep_parity` findings and \"Keep Every GN Dependency Label and Edge Kind\" in the coder "
                           "instructions), re-run `fx bazel2gn -d <dir>` and `fx build <failing label>`. Never edit sources "
                           "or the dependents' BUILD files to make it pass.",
    "bazel2gn_verifications": "BUILD.gn no longer matches BUILD.bazel. Run `fx bazel2gn -d <dir>` for every dual-build directory "
                              "you touched (never hand-edit below the BAZEL2GN SENTINEL) and re-run `fx build --host //build:bazel2gn_verifications`.",
    "bazel_build_fuchsia": "Fix BUILD.bazel so `fx bazel build --config=fuchsia_platform //<dir>:all` passes. If a dependency has no "
                           "Bazel target yet, migrate that dependency's directory in this change (see the coder instructions).",
    "bazel_build_host": "Fix BUILD.bazel so `fx bazel build --config=host //<dir>:all` passes. If a dependency has no Bazel target "
                        "yet, migrate that dependency's directory in this change (see the coder instructions).",
}

findings = []
if change_base != "HEAD":
    toolchains_map, _ = load_toolchains()
    if toolchains_map is not None:
        commit_msg = "\n".join(git_lines(["log", "-1", "--format=%B", "HEAD"]))
        for m_line in commit_msg.splitlines():
            if re.match(r"^\s*Test\s*:", m_line, re.I) and "fx build" in m_line:
                if "--host" in m_line and "//build:bazel2gn_verifications" in m_line:
                    continue
                bad_lbls = []
                for lbl in re.findall(r"//[^\s,;\"'`]+", m_line):
                    base_lbl = lbl.split("(", 1)[0]
                    if base_lbl not in toolchains_map and lbl not in toolchains_map.get(base_lbl, []):
                        bad_lbls.append(lbl)
                if bad_lbls:
                    findings.append({
                        "source": "build_verification",
                        "category": "unconfigured_test_footer_label",
                        "severity": "ERROR",
                        "file": "",
                        "line": 0,
                        "message": (
                            f"Commit `Test:` footer `{m_line.strip()}` passes GN label(s) not in the configured GN graph "
                            f"(<build dir>/ninja_outputs.json): {' '.join(bad_lbls)}. `fx build` fails with `Unknown GN label` "
                            "on labels outside the configured product graph."
                        ),
                        "remediation": (
                            "Amend the commit message `Test:` footer (and only list commands in `CoderReport.tests_run`) to use "
                            "commands that actually succeed in the configured build (such as `fx build`, "
                            "`fx build --host //build:bazel2gn_verifications`, `fx bazel build --config=fuchsia_platform //<dir>:all`, "
                            "and `fx build <configured_label>` from `gn_build_dependents`)."
                        ),
                    })

failed = set()
for name, cmd in steps:
    if name == "gn_build_dependents":
        if "gn_build" in failed:
            continue
        labels, notes = dependent_labels()
        findings.extend(
            {"source": "build_verification", "category": "dependents_note", "severity": "INFO", "message": n} for n in notes
        )
        if not labels:
            continue
        cmd = [fx, "build"] + labels
    if dry_run:
        findings.append({
            "source": "build_verification",
            "category": "build_dry_run",
            "severity": "INFO",
            "message": f"PLANTER_BUILD_DRY_RUN: {name} would run `{' '.join(['fx'] + cmd[1:])}`",
        })
        continue
    try:
        proc = subprocess.run(cmd, cwd=workdir, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=timeout)
        code, output = proc.returncode, proc.stdout
    except subprocess.TimeoutExpired as e:
        code = "timeout"
        output = (e.stdout or b"").decode(errors="replace") if isinstance(e.stdout, bytes) else (e.stdout or "")
        output += f"\n(timed out after {timeout}s)"
    if code == 0:
        continue
    failed.add(name)
    text, file, line = summarize(output, BUILD_FILE_LOCATION if name == "gn_build_dependents" else LOCATION)
    findings.append({
        "source": "build_verification",
        "category": f"build_failure_{name}",
        "severity": "ERROR",
        "file": file,
        "line": line,
        "message": f"`{' '.join(['fx'] + cmd[1:])}` failed ({code}):\n{text}",
        "remediation": REMEDIATION[name],
    })
    if name == "gn_build" and re.search(r"fx set|no build directory|Unable to find build directory", output, re.I):
        break  # The tree is not configured; the remaining steps would fail the same way.

print(json.dumps(findings, indent=2))
PYEOF
