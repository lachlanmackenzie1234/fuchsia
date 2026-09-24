#!/usr/bin/env bash
# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

set -euo pipefail

# Helper tool to discover true reverse dependencies (deps, public_deps, test_deps, proc_macro_deps, actual)
# of a target package directory while filtering out references inside visibility = [...] or visibility.gni.

WORKDIR="${PLANTER_WORKDIR:-.}"
TARGET_DIR="${1:-${PLANTER_TARGET_DIR:-}}"

if [[ -z "$TARGET_DIR" ]]; then
  echo "Usage: find_rdeps.sh <target_package_dir>" >&2
  exit 1
fi

python3 - "$WORKDIR" "$TARGET_DIR" <<'PYEOF'
import json
import os
import re
import subprocess
import sys

workdir = os.path.abspath(sys.argv[1])
target_pkg = sys.argv[2].strip().strip("/")
label_prefix = f"//{target_pkg}"

cmd = ["rg", "-l", "--fixed-strings", label_prefix, "-g", "*BUILD.gn", "-g", "*BUILD.bazel", "-g", "*.gni", workdir]
try:
    out = subprocess.check_output(cmd, stderr=subprocess.DEVNULL, text=True)
    files = [line.strip() for line in out.splitlines() if line.strip()]
except Exception:
    files = []

true_rdep_packages = set()
visibility_only_packages = set()

for fpath in sorted(files):
    rel = os.path.relpath(fpath, workdir)
    pkg = os.path.dirname(rel).strip("/")
    if pkg == target_pkg:
        continue
    if os.path.basename(rel) == "visibility.gni":
        visibility_only_packages.add(pkg)
        continue
    try:
        with open(fpath, "r", encoding="utf-8", errors="ignore") as f:
            content = f.read()
    except Exception:
        continue
    stripped = re.sub(r"visibility\s*\+?=\s*\[[^\]]*\]", "", content, flags=re.DOTALL)
    if label_prefix in stripped:
        true_rdep_packages.add(pkg)
    elif label_prefix in content:
        visibility_only_packages.add(pkg)

visibility_only_packages -= true_rdep_packages

print(json.dumps({
    "target_package": f"//{target_pkg}",
    "true_rdep_packages": sorted(f"//{p}" for p in true_rdep_packages),
    "excluded_visibility_allowlist_packages": sorted(f"//{p}" for p in visibility_only_packages),
}, indent=2))
PYEOF
