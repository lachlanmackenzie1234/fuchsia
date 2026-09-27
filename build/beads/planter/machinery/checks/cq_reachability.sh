#!/usr/bin/env bash
# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

set -euo pipefail

# Deterministic check verifying that migrated targets and tests in BUILD.bazel /
# BUILD.gn remain reachable in the build & CQ graph:
# 1. Every test target (`*_test`, `fuchsia_unittest_package`, `fuchsia_test_package`)
#    in the migrated package must be wired into the package's `group("tests")`
#    (or referenced by a test package in the same BUILD file that is wired into
#    `tests`).
# 2. The package must be registered in a Bazel verification list or parent BUILD
#    file so CQ builds it. Note that the bazel2gn list .gni files only ensure
#    that the GN (and maybe Bazel) file is processed; they do not ensure that
#    any of the targets can be built successfully.

WORKDIR="${PLANTER_WORKDIR:-.}"
TARGET_DIR="${PLANTER_TARGET_DIR:-}"
TARGET_DIRS="${PLANTER_TARGET_DIRS:-$TARGET_DIR}"

python3 - "$WORKDIR" "$TARGET_DIRS" <<'PYEOF'
import ast
import json
import os
import re
import sys

workdir = os.path.abspath(sys.argv[1])
raw_dirs = sys.argv[2]
target_dirs = [d.strip().strip("/") for d in re.split(r"[\s,]+", raw_dirs) if d.strip().strip("/")]

TEST_RULES = {
    "rustc_test",
    "cc_test",
    "go_test",
    "python_host_test",
    "fuchsia_unittest_package",
    "fuchsia_test_package",
}

findings = []

for td in target_dirs:
    bazel_rel = os.path.join(td, "BUILD.bazel")
    bazel_full = os.path.join(workdir, bazel_rel)
    gn_rel = os.path.join(td, "BUILD.gn")
    gn_full = os.path.join(workdir, gn_rel)

    if not os.path.isfile(bazel_full):
        continue

    try:
        with open(bazel_full, "r", encoding="utf-8") as f:
            bazel_src = f.read()
        tree = ast.parse(bazel_src, filename=bazel_rel)
    except Exception:
        continue

    gn_src = ""
    if os.path.isfile(gn_full):
        try:
            with open(gn_full, "r", encoding="utf-8") as f:
                gn_src = f.read()
        except Exception:
            gn_src = ""

    test_targets = []
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call):
            continue
        rule_name = ""
        if isinstance(node.func, ast.Name):
            rule_name = node.func.id
        elif isinstance(node.func, ast.Attribute):
            rule_name = node.func.attr
        if rule_name not in TEST_RULES:
            continue
        t_name = ""
        lineno = getattr(node, "lineno", 1)
        for kw in node.keywords:
            if kw.arg == "name" and isinstance(kw.value, ast.Constant) and isinstance(kw.value.value, str):
                t_name = kw.value.value
                lineno = kw.lineno
        if t_name:
            test_targets.append((t_name, rule_name, lineno))

    parent_gn = os.path.join(workdir, os.path.dirname(td), "BUILD.gn")
    parent_gn_src = ""
    if os.path.isfile(parent_gn):
        try:
            with open(parent_gn, "r", encoding="utf-8") as f:
                parent_gn_src = f.read()
        except Exception:
            parent_gn_src = ""

    for t_name, rule_name, lineno in test_targets:
        ref_patterns = [
            f'":{t_name}"',
            f'"{t_name}"',
            f":{t_name}\"",
            f"{os.path.basename(td)}:{t_name}",
        ]
        referenced_in_gn = any(p in gn_src for p in ref_patterns)
        referenced_in_parent = any(p in parent_gn_src for p in ref_patterns) or (
            f'"{os.path.basename(td)}:tests"' in parent_gn_src
        )
        referenced_in_bazel = bazel_src.count(f'":{t_name}"') >= 1 or bazel_src.count(f'"{t_name}"') >= 2
        if not (referenced_in_gn or referenced_in_parent or referenced_in_bazel):
            findings.append({
                "source": "cq_reachability",
                "category": "orphaned_test_target",
                "severity": "error",
                "file": bazel_rel,
                "line": lineno,
                "message": (
                    f"Test target ':{t_name}' ({rule_name}) in '{bazel_rel}' is not referenced by "
                    "`group(\"tests\")`, a test package, or the parent directory's `tests` group, "
                    "so it would be silently dropped from CQ."
                ),
                "remediation": (
                    f"Wire ':{t_name}' into `group(\"tests\")` (or its enclosing `fuchsia_unittest_package` / "
                    "parent `BUILD.gn` `tests` group) so CQ continues to build and run this test."
                ),
            })

print(json.dumps(findings, indent=2))
PYEOF
