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


# Names of host_test_data_files / host_test_data_map targets in package `pkg`.
def host_test_data_names(pkg):
    if pkg not in _htd_cache:
        names = set()
        p = os.path.join(workdir, pkg, "BUILD.bazel")
        if os.path.isfile(p):
            t = open(p, encoding="utf-8").read()
            for hm in HOST_TEST_DATA_RULE_RE.finditer(t):
                nm = re.search(r'^\s+name\s*=\s*"([^"]+)"', block_at(t, hm.end() - 1), re.M)
                if nm:
                    names.add(nm.group(1))
        _htd_cache[pkg] = names
    return _htd_cache[pkg]


def is_host_test_data_label(label, pkg):
    if label.startswith(":"):
        lpkg, name = pkg, label[1:]
    elif label.startswith("//") and ":" in label:
        lpkg, name = label[2:].split(":", 1)
    else:
        return False
    return name in host_test_data_names(lpkg)


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
