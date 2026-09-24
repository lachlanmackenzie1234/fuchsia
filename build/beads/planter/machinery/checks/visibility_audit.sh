#!/usr/bin/env bash
# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

set -euo pipefail

# Deterministic check auditing BUILD.bazel files for visibility scoping and rdeps anti-patterns:
# 1. Package-level default_visibility in package(...)
# 2. Redundant ':__pkg__' (or '//<current_pkg>:__pkg__') in target visibility lists
# 3. Disguised repository-wide visibility ('//:__subpackages__')
# 4. Pseudo-public visibility lists enumerating >= 5 top-level '//<dir>:__subpackages__' roots
# 5. False-positive rdeps where the granted package only mentions this package inside a visibility allowlist (.gni / visibility = [...])

WORKDIR="${PLANTER_WORKDIR:-.}"
TARGET_DIR="${PLANTER_TARGET_DIR:-}"

python3 - "$WORKDIR" "$TARGET_DIR" <<'PYEOF'
import ast
import json
import os
import re
import subprocess
import sys

workdir = os.path.abspath(sys.argv[1])
target_dir = sys.argv[2].strip().strip("/")

candidate_files = set()
if target_dir:
    rel_bazel = os.path.join(target_dir, "BUILD.bazel")
    if os.path.isfile(os.path.join(workdir, rel_bazel)):
        candidate_files.add(rel_bazel)

try:
    out = subprocess.check_output(
        ["git", "-C", workdir, "diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"],
        stderr=subprocess.DEVNULL,
        text=True,
    )
    for line in out.splitlines():
        line = line.strip()
        if line.endswith("BUILD.bazel") and os.path.isfile(os.path.join(workdir, line)):
            candidate_files.add(line)
except Exception:
    pass

def has_only_visibility_mention(caller_pkg: str, target_pkg: str) -> bool:
    """Returns True if caller_pkg only mentions //target_pkg inside visibility lists or visibility.gni."""
    caller_dir = os.path.join(workdir, caller_pkg)
    if not os.path.isdir(caller_dir):
        return False
    vis_gni = os.path.join(caller_dir, "visibility.gni")
    build_gn = os.path.join(caller_dir, "BUILD.gn")
    build_bazel = os.path.join(caller_dir, "BUILD.bazel")

    target_prefix = f"//{target_pkg}"
    mentioned_in_vis = False
    if os.path.isfile(vis_gni):
        try:
            with open(vis_gni, "r", encoding="utf-8", errors="ignore") as f:
                if target_prefix in f.read():
                    mentioned_in_vis = True
        except Exception:
            pass

    dep_mention = False
    for bfile in (build_gn, build_bazel):
        if not os.path.isfile(bfile):
            continue
        try:
            with open(bfile, "r", encoding="utf-8", errors="ignore") as f:
                content = f.read()
        except Exception:
            continue
        if target_prefix not in content:
            continue
        stripped = re.sub(r"visibility\s*\+?=\s*\[[^\]]*\]", "", content, flags=re.DOTALL)
        if target_prefix in stripped:
            dep_mention = True
        else:
            mentioned_in_vis = True

    return mentioned_in_vis and not dep_mention

findings = []

for rel_path in sorted(candidate_files):
    full_path = os.path.join(workdir, rel_path)
    pkg_path = os.path.dirname(rel_path).strip("/")
    try:
        with open(full_path, "r", encoding="utf-8") as f:
            src = f.read()
        tree = ast.parse(src, filename=rel_path)
    except Exception:
        continue

    for node in ast.walk(tree):
        if not isinstance(node, ast.Call):
            continue
        func_name = ""
        if isinstance(node.func, ast.Name):
            func_name = node.func.id
        elif isinstance(node.func, ast.Attribute):
            func_name = node.func.attr

        if func_name == "package":
            for kw in node.keywords:
                if kw.arg == "default_visibility":
                    findings.append({
                        "source": "visibility_audit",
                        "category": "visibility_scoping",
                        "severity": "error",
                        "file": rel_path,
                        "line": kw.lineno,
                        "message": "Do not set package-level 'default_visibility' in package(...). Declare visibility explicitly on individual targets.",
                        "remediation": "Remove default_visibility from package() and set visibility on specific targets."
                    })
            continue

        target_name = "<unnamed>"
        for kw in node.keywords:
            if kw.arg == "name" and isinstance(kw.value, ast.Constant) and isinstance(kw.value.value, str):
                target_name = kw.value.value

        for kw in node.keywords:
            if kw.arg != "visibility" or not isinstance(kw.value, ast.List):
                continue
            entries = []
            for elt in kw.value.elts:
                if isinstance(elt, ast.Constant) and isinstance(elt.value, str):
                    entries.append((elt.value, getattr(elt, "lineno", kw.lineno)))

            top_level_subpkgs = []
            for val, lineno in entries:
                if val in (":__pkg__", f"//{pkg_path}:__pkg__"):
                    findings.append({
                        "source": "visibility_audit",
                        "category": "redundant_same_package_visibility",
                        "severity": "error",
                        "file": rel_path,
                        "line": lineno,
                        "message": f"Target '{target_name}' includes redundant '{val}' in visibility. Same-package visibility is always implicit in Bazel.",
                        "remediation": "Remove ':__pkg__' from BUILD.bazel (prioritize Bazel-correctness; if GN requires ':*', adjust bazel2gn to generate it)."
                    })
                elif val == "//:__subpackages__":
                    findings.append({
                        "source": "visibility_audit",
                        "category": "disguised_public_visibility",
                        "severity": "error",
                        "file": rel_path,
                        "line": lineno,
                        "message": f"Target '{target_name}' uses '//:__subpackages__' in visibility, which is a disguised repository-wide wildcard.",
                        "remediation": "Scope visibility to actual reverse-dependency packages/subpackages, or use '//visibility:public' if this is genuinely a tree-wide public API."
                    })
                else:
                    m_top = re.match(r"^//([^/:]+):__subpackages__$", val)
                    if m_top:
                        top_level_subpkgs.append((val, lineno))

                    m_pkg = re.match(r"^//([^:]+):__(?:pkg|subpackages)__$", val)
                    if m_pkg:
                        caller_pkg = m_pkg.group(1).strip("/")
                        if has_only_visibility_mention(caller_pkg, pkg_path):
                            findings.append({
                                "source": "visibility_audit",
                                "category": "visibility_allowlist_false_rdep",
                                "severity": "error",
                                "file": rel_path,
                                "line": lineno,
                                "message": f"Target '{target_name}' grants visibility to '{val}', but '//{caller_pkg}' only references '//{pkg_path}' inside its own visibility allowlist (not as an actual dependency).",
                                "remediation": f"Remove '{val}' from visibility; only include packages that depend on '{target_name}' in deps/public_deps/test_deps."
                            })

            if len(top_level_subpkgs) >= 5:
                findings.append({
                    "source": "visibility_audit",
                    "category": "disguised_public_visibility",
                    "severity": "error",
                    "file": rel_path,
                    "line": kw.lineno,
                    "message": f"Target '{target_name}' enumerates {len(top_level_subpkgs)} top-level '//<dir>:__subpackages__' roots in visibility instead of declaring '//visibility:public' or scoping to specific callers.",
                    "remediation": "If this target is used across the tree as a public API, set visibility = [\"//visibility:public\"]; otherwise scope visibility to the specific subpackages that depend on it."
                })

print(json.dumps(findings, indent=2))
PYEOF
