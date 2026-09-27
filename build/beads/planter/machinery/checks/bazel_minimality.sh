#!/usr/bin/env bash
# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

set -euo pipefail

# Deterministic check enforcing BUILD.bazel minimality and strict target-directory
# scope:
# 1. Flags redundant default attributes in BUILD.bazel:
#    - crate_name = "<x>" when identical to target name (modulo '-' vs '_')
#    - version = "0.1.0" on first-party (in-tree) rustc_* targets
#    - crate_root = "src/lib.rs" on rustc_library
#    - crate_root = "src/main.rs" on rustc_binary
# 2. Flags drive-by edits to unrelated packages outside PLANTER_TARGET_DIR /
#    PLANTER_TARGET_DIRS (except centralized registration lists or direct parent
#    BUILD.gn/BUILD.bazel test groups).
#    TODO: Evaluate each out-of-scope edit for validity/usefulness and apply or
#    batch valid changes in a separate commit.

WORKDIR="${PLANTER_WORKDIR:-.}"
TARGET_DIR="${PLANTER_TARGET_DIR:-}"
TARGET_DIRS="${PLANTER_TARGET_DIRS:-$TARGET_DIR}"
CHANGE_BASE="${PLANTER_CHANGE_BASE:-HEAD}"

python3 - "$WORKDIR" "$TARGET_DIRS" "$CHANGE_BASE" <<'PYEOF'
import ast
import json
import os
import re
import subprocess
import sys

workdir = os.path.abspath(sys.argv[1])
raw_dirs = sys.argv[2]
change_base = sys.argv[3].strip() or "HEAD"
target_dirs = [d.strip().strip("/") for d in re.split(r"[\s,]+", raw_dirs) if d.strip().strip("/")]

ALLOWED_GLOBAL_PREFIXES = (
    "build/bazel2gn",
    "build/bazel/",
)


def git_lines(args):
    try:
        out = subprocess.check_output(
            ["git", "-C", workdir] + args, stderr=subprocess.DEVNULL, text=True
        )
    except Exception:
        return []
    return [l.strip() for l in out.splitlines() if l.strip()]


changed = set()
changed.update(git_lines(["diff", "--name-only", change_base]))
changed.update(git_lines(["ls-files", "--others", "--exclude-standard"]))


def is_allowed_scope(path: str) -> bool:
    if not target_dirs:
        return True
    norm = path.strip("/")
    pkg_dir = os.path.dirname(norm)
    for td in target_dirs:
        if norm == td or norm.startswith(td + "/"):
            return True
        if pkg_dir == os.path.dirname(td) and os.path.basename(norm) in ("BUILD.gn", "BUILD.bazel"):
            return True
    for prefix in ALLOWED_GLOBAL_PREFIXES:
        if norm.startswith(prefix):
            return True
    if norm in ("tools/BUILD.gn", "src/BUILD.gn", "sdk/BUILD.gn"):
        return True
    return False


findings = []

for path in sorted(changed):
    if not is_allowed_scope(path):
        findings.append({
            "source": "bazel_minimality",
            "category": "out_of_scope_package_modified",
            "severity": "error",
            "file": path,
            "line": 1,
            "message": (
                f"File '{path}' is outside the assigned target directory scope "
                f"({', '.join(target_dirs)}). Drive-by edits or cleanups in unrelated packages are forbidden."
            ),
            "remediation": (
                f"Revert changes to '{path}' (`git checkout {change_base} -- {path}` or `git checkout -- {path}`) "
                "and restrict modifications strictly to the assigned target package(s)."
            ),
        })

bazel_files = []
for td in target_dirs:
    cand = os.path.join(workdir, td, "BUILD.bazel")
    if os.path.isfile(cand):
        bazel_files.append((os.path.join(td, "BUILD.bazel"), cand))

for rel_path, full_path in bazel_files:
    try:
        with open(full_path, "r", encoding="utf-8") as f:
            source = f.read()
        tree = ast.parse(source, filename=rel_path)
    except Exception:
        continue

    for node in ast.walk(tree):
        if not isinstance(node, ast.Call):
            continue
        rule_name = ""
        if isinstance(node.func, ast.Name):
            rule_name = node.func.id
        elif isinstance(node.func, ast.Attribute):
            rule_name = node.func.attr
        if not rule_name:
            continue

        kw_map = {}
        for kw in node.keywords:
            if kw.arg and isinstance(kw.value, ast.Constant) and isinstance(kw.value.value, str):
                kw_map[kw.arg] = (kw.value.value, kw.lineno)

        target_name, _ = kw_map.get("name", ("", 0))

        # 1. Redundant crate_name matching name (or name with '-' -> '_')
        if "crate_name" in kw_map and target_name:
            c_name, lineno = kw_map["crate_name"]
            if c_name == target_name or c_name == target_name.replace("-", "_"):
                findings.append({
                    "source": "bazel_minimality",
                    "category": "redundant_default_attribute",
                    "severity": "error",
                    "file": rel_path,
                    "line": lineno,
                    "message": (
                        f"Target '{target_name}' ({rule_name}) specifies redundant `crate_name = \"{c_name}\"`, "
                        "which is already the default derived from `name`."
                    ),
                    "remediation": (
                        "Remove `crate_name` from BUILD.bazel and re-run `fx bazel2gn` so the target definition stays minimal."
                    ),
                })

        # 2. Redundant version = "0.1.0" on rustc_* targets
        if rule_name.startswith("rustc_") and "version" in kw_map:
            ver, lineno = kw_map["version"]
            if ver in ("0.1.0", "0.0.1"):
                findings.append({
                    "source": "bazel_minimality",
                    "category": "redundant_default_attribute",
                    "severity": "error",
                    "file": rel_path,
                    "line": lineno,
                    "message": (
                        f"Target '{target_name}' ({rule_name}) specifies redundant `version = \"{ver}\"`, "
                        "which defaults to \"0.1.0\" in Fuchsia's Rust Bazel rules."
                    ),
                    "remediation": (
                        "Remove `version` from BUILD.bazel (and from BUILD.gn if dual-building) and re-run `fx bazel2gn`."
                    ),
                })

        # 3. Redundant default crate_root on rustc_library / rustc_binary
        if "crate_root" in kw_map:
            c_root, lineno = kw_map["crate_root"]
            if (rule_name == "rustc_library" and c_root == "src/lib.rs") or (
                rule_name == "rustc_binary" and c_root == "src/main.rs"
            ):
                findings.append({
                    "source": "bazel_minimality",
                    "category": "redundant_default_attribute",
                    "severity": "error",
                    "file": rel_path,
                    "line": lineno,
                    "message": (
                        f"Target '{target_name}' ({rule_name}) specifies redundant `crate_root = \"{c_root}\"`, "
                        f"which is the default for `{rule_name}`."
                    ),
                    "remediation": (
                        "Remove `crate_root` from BUILD.bazel and re-run `fx bazel2gn`."
                    ),
                })

print(json.dumps(findings, indent=2))
PYEOF
