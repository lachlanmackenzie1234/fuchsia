#!/usr/bin/env bash
# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

set -euo pipefail

# Flags test-only data files placed in the runtime `data` attribute of a
# rustc_library/rustc_binary/rustc_proc_macro that defines unit tests
# (with_unit_tests / with_host_unit_tests). Files needed only by the generated
# <name>_test belong in `test_data` (or a host_test_data() target from
# //build/bazel/rules/host_tests/host_test_data.bzl for host tests), not in
# `data`, which ships them with the library and forces rule macros (e.g. host
# test wrappers) to be edited to forward `data`.
#
# Also flags host_test_data_files / host_test_data_map targets listed in `data`
# of a host_rustc_test (host_test_data_in_data): runtime test data belongs in
# `test_data`, which host_rustc_test forwards to its host_test() wrapper.

WORKDIR="${PLANTER_WORKDIR:-.}"
TARGET_DIR="${PLANTER_TARGET_DIR:-}"

python3 - "$WORKDIR" "$TARGET_DIR" <<'PYEOF'
import json
import os
import re
import sys

workdir = os.path.abspath(sys.argv[1])
target_dirs = [d.strip().strip("/") for d in re.split(r"[,\s]+", sys.argv[2]) if d.strip()]

RULE_RE = re.compile(r"^(rustc_library|rustc_binary|rustc_proc_macro)\(\s*$", re.M)
TESTY = re.compile(r"(^|[/_.-])(test_?data|testdata|tests?|fixtures?|golden)([/_.-]|$)", re.I)

findings = []


# Emit a standardized check finding entry.
def _emit(findings, category, severity, file, line, message, remediation=""):
    findings.append(
        {
            "source": "test_data_attribute",
            "category": category,
            "severity": severity,
            "file": file,
            "line": line,
            "message": message,
            "remediation": remediation,
        }
    )


# Extract a balanced parenthesized rule call block starting at `start`.
def block_at(text, start):
    depth = 0
    for i in range(start, len(text)):
        c = text[i]
        if c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
            if depth == 0:
                return text[start:i + 1]
    return text[start:]


def list_attr(block, attr):
    m = re.search(r"^\s+" + attr + r"\s*=\s*\[(.*?)\]", block, re.M | re.S)
    if not m:
        return None
    return re.findall(r'"([^"]+)"', m.group(1))


HOST_RUST_TEST_RE = re.compile(r"^host_rustc_test\(\s*$", re.M)
HOST_TEST_DATA_RULE_RE = re.compile(r"^(host_test_data_files|host_test_data_map)\(\s*$", re.M)
_htd_cache = {}


# Names and staged path tokens of host_test_data_files / host_test_data_map targets in `pkg`.
_GENERIC_SEGMENTS = {
    "test_data",
    "testdata",
    "data",
    "host_test_data",
    "keys",
    "repository",
    "blobs",
    "meta",
    "bin",
    "lib",
}


def host_test_data_info(pkg):
    if pkg not in _htd_cache:
        info = {}
        p = os.path.join(workdir, pkg, "BUILD.bazel")
        if os.path.isfile(p):
            t = open(p, encoding="utf-8").read()
            for hm in HOST_TEST_DATA_RULE_RE.finditer(t):
                blk = block_at(t, hm.end() - 1)
                nm = re.search(r'^\s+name\s*=\s*"([^"]+)"', blk, re.M)
                if not nm:
                    continue
                tokens = set()
                dm = re.search(r'^\s+dest_dir\s*=\s*"([^"]+)"', blk, re.M)
                if dm:
                    dd = dm.group(1).strip().strip("/")
                    if dd:
                        tokens.add(dd)
                        parent = os.path.dirname(dd)
                        if parent and parent not in _GENERIC_SEGMENTS:
                            tokens.add(parent)
                        for seg in dd.split("/"):
                            if seg and seg not in _GENERIC_SEGMENTS:
                                tokens.add(seg)
                for src in list_attr(blk, "srcs") or []:
                    base = os.path.basename(src.split(":", 1)[-1]).strip()
                    if base and not tokens:
                        tokens.add(base)
                info[nm.group(1)] = tokens
        _htd_cache[pkg] = info
    return _htd_cache[pkg]


def host_test_data_names(pkg):
    return set(host_test_data_info(pkg).keys())


def parse_host_test_data_label(label, pkg):
    if label.startswith(":"):
        lpkg, name = pkg, label[1:]
    elif label.startswith("//") and ":" in label:
        lpkg, name = label[2:].split(":", 1)
    else:
        return None
    if name in host_test_data_names(lpkg):
        return lpkg, name
    return None


def is_host_test_data_label(label, pkg):
    return parse_host_test_data_label(label, pkg) is not None


# Read all non-BUILD source files in package `pkg` to verify test_data consumption.
def package_source_blob(pkg):
    chunks = []
    root = os.path.join(workdir, pkg)
    if not os.path.isdir(root):
        return ""
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [
            dn
            for dn in dirnames
            if not dn.startswith(".")
            and not os.path.isfile(os.path.join(dirpath, dn, "BUILD.bazel"))
        ]
        for fn in filenames:
            if fn in ("BUILD.bazel", "BUILD.gn", "BUILD"):
                continue
            fpath = os.path.join(dirpath, fn)
            try:
                chunks.append(open(fpath, encoding="utf-8", errors="ignore").read())
            except OSError:
                pass
    return "\n".join(chunks)


# Check that host_test_data targets listed in `test_data` of a host_rustc_test are actually
# referenced by at least one source or test file in the package.
def check_unused_host_test_data(text, d):
    src_blob = None
    for m in HOST_RUST_TEST_RE.finditer(text):
        block = block_at(text, m.end() - 1)
        td_entries = list_attr(block, "test_data") or []
        if not td_entries:
            continue
        name_m = re.search(r'^\s+name\s*=\s*"([^"]+)"', block, re.M)
        name = name_m.group(1) if name_m else "?"
        unused = []
        for entry in td_entries:
            parsed = parse_host_test_data_label(entry, d)
            if not parsed:
                continue
            lpkg, htd_name = parsed
            tokens = host_test_data_info(lpkg).get(htd_name) or set()
            if not tokens:
                continue
            if src_blob is None:
                src_blob = package_source_blob(d)
            if not any(tok in src_blob for tok in tokens):
                unused.append((entry, lpkg, htd_name))
        if not unused:
            continue
        labels_desc = ", ".join(u[0] for u in unused)
        cross_pkgs = sorted({u[1] for u in unused if u[1] != d})
        vis_note = (
            f" Also remove `\"//{d}:__pkg__\"` from the `visibility` list of those "
            f"`host_test_data_*` target(s) in {', '.join(f'{p}/BUILD.bazel' for p in cross_pkgs)}."
            if cross_pkgs
            else ""
        )
        _emit(
            findings,
            "unused_test_data",
            "ERROR",
            os.path.join(d, "BUILD.bazel"),
            text[: m.start()].count("\n") + 1,
            (
                f'host_rustc_test(name = "{name}") specifies `test_data` entries '
                f"({labels_desc}), but no source or test file in //{d} references or "
                "consumes their staged test data paths."
            ),
            (
                f"Remove the unused `test_data` entry/attribute ({labels_desc}) from "
                f'`host_rustc_test(name = "{name}")` in {d}/BUILD.bazel (and update '
                "any skip comment above it that mentions `test_data`)."
                + vis_note
                + " If those targets were in GN `test_deps`, note their removal in "
                "`CoderReport.summary` and dispute any resulting "
                "`review_diff/unit_test_parity` finding in "
                "`CoderReport.disputed_findings` once all deterministic check "
                "scripts pass."
            ),
        )


# host_rustc_test forwards `test_data` to its host_test() wrapper; `data` only
# reaches the inner rustc_test binary. Runtime test data belongs in `test_data`.
def check_host_rustc_test_data(text, d):
    for m in HOST_RUST_TEST_RE.finditer(text):
        block = block_at(text, m.end() - 1)
        bad = [e for e in (list_attr(block, "data") or []) if is_host_test_data_label(e, d)]
        if not bad:
            continue
        name_m = re.search(r'^\s+name\s*=\s*"([^"]+)"', block, re.M)
        name = name_m.group(1) if name_m else "?"
        _emit(
            findings,
            "host_test_data_in_data",
            "ERROR",
            os.path.join(d, "BUILD.bazel"),
            text[:m.start()].count("\n") + 1,
            (
                f"host_rustc_test(name = \"{name}\") lists host test data targets in `data`: "
                f"{', '.join(bad)}. `data` only reaches the inner rustc_test binary; runtime "
                "test data belongs in `test_data`, which is forwarded to the host_test() wrapper."
            ),
            (
                "Move these labels from `data` to `test_data`. bazel2gn cannot translate "
                "`test_data` on host_rustc_test, so mark the test `# @bazel2gn:skip` with an "
                "explanatory comment (migration_sanity T3), export it via the nearest ancestor "
                "`test_suite` (`host_tests`), and drop GN host_test_data targets above the "
                "sentinel that no longer have GN consumers."
            ),
        )


for d in target_dirs:
    path = os.path.join(workdir, d, "BUILD.bazel")
    if not os.path.isfile(path):
        continue
    text = open(path, encoding="utf-8").read()
    check_host_rustc_test_data(text, d)
    check_unused_host_test_data(text, d)
    for m in RULE_RE.finditer(text):
        block = block_at(text, m.end() - 1)
        if not re.search(r"^\s+with_(host_)?unit_tests\s*=", block, re.M):
            continue
        name_m = re.search(r'^\s+name\s*=\s*"([^"]+)"', block, re.M)
        name = name_m.group(1) if name_m else "?"
        data = list_attr(block, "data") or []
        testy = [e for e in data if TESTY.search(e)]
        if not testy:
            continue
        line = text[:m.start()].count("\n") + 1
        _emit(
            findings,
            "test_only_data_in_data",
            "ERROR",
            os.path.join(d, "BUILD.bazel"),
            line,
            (
                f"{m.group(1)}(name = \"{name}\") defines unit tests but lists test-only "
                f"files in `data`: {', '.join(testy)}. `data` is runtime data of the "
                "library itself and ships to every dependent."
            ),
            (
                "Move files used only by the generated <name>_test to `test_data` (for host "
                "tests, a host_test_data() target from "
                "//build/bazel/rules/host_tests/host_test_data.bzl when the test needs them at "
                "a stable runtime path). Do not edit shared rule macros (.bzl) to forward "
                "`data` to work around this, and do not change sources to locate the files. "
                "Example: data = [\"test_data/main.json\"] -> test_data = [\"test_data/main.json\"]."
            ),
        )

print(json.dumps(findings, indent=2))
PYEOF
