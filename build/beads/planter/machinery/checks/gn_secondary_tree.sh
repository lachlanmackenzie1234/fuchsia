#!/usr/bin/env bash
# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

set -euo pipefail

# Rejects Bazel build files added or modified under build/secondary/ and
# bazel2gn target-map entries pointing into build/secondary/.
#
# build/secondary/ is GN's secondary source tree: GN-only overlay BUILD.gn files
# for third-party code that ships without one. Bazel has no equivalent, so a
# BUILD.bazel there is not transparent to callers and requires extra label
# mappings. The fix is the current third-party layout: move the code to
# //third_party/<name>/src and put BUILD.gn + BUILD.bazel (plus README.fuchsia
# and LICENSE as required) in //third_party/<name>/.

WORKDIR="${PLANTER_WORKDIR:-.}"

python3 - "$WORKDIR" <<'PYEOF'
import json
import os
import subprocess
import sys

workdir = os.path.abspath(sys.argv[1])
change_base = os.environ.get("PLANTER_CHANGE_BASE", "").strip() or "HEAD"


def git(args):
    try:
        return subprocess.check_output(
            ["git", "-C", workdir] + args, stderr=subprocess.DEVNULL, text=True
        )
    except Exception:
        return ""


changed = {l.strip() for l in git(["diff", "--name-only", "--diff-filter=AM", change_base]).splitlines() if l.strip()}
changed.update(l.strip() for l in git(["ls-files", "--others", "--exclude-standard"]).splitlines() if l.strip())

REMEDIATION = (
    "build/secondary/ is a GN-only overlay tree; do not add Bazel builds there. Migrate the "
    "third-party code to the current layout instead: move its sources to //third_party/<name>/src, "
    "add BUILD.gn and BUILD.bazel (plus README.fuchsia and LICENSE if needed) under "
    "//third_party/<name>/, reuse an existing upstream BUILD file there when it matches GN, and "
    "point callers at the same label for GN and Bazel so no bazel2gn target-map entry is needed "
    "(see docs/development/source_code/third-party-management.md, 'Migrating legacy third-party "
    "code to current layout'). If that move is out of scope, stop and report the blocker in the summary."
)

findings = []
for path in sorted(changed):
    base = os.path.basename(path)
    if path.startswith("build/secondary/") and (
        base in ("BUILD.bazel", "BUILD", "MODULE.bazel")
        or base.endswith((".bzl", ".BUILD.bazel"))
    ):
        findings.append({
            "source": "gn_secondary_tree",
            "category": "bazel_build_in_gn_secondary_tree",
            "severity": "error",
            "file": path,
            "line": 0,
            "message": f"Change adds or edits Bazel build file '{path}' inside GN's secondary source tree.",
            "remediation": REMEDIATION,
        })

for path in sorted(changed):
    if not path.endswith("third_party_target_map.json"):
        continue
    diff = git(["diff", "-U0", change_base, "--", path])
    if not diff and path in changed:
        try:
            diff = "".join("+" + l for l in open(os.path.join(workdir, path)))
        except OSError:
            diff = ""
    for line in diff.splitlines():
        if line.startswith("+") and not line.startswith("+++") and "build/secondary" in line:
            findings.append({
                "source": "gn_secondary_tree",
                "category": "target_map_points_into_gn_secondary_tree",
                "severity": "error",
                "file": path,
                "line": 0,
                "message": f"Added bazel2gn target-map entry references build/secondary: {line[1:].strip()}",
                "remediation": REMEDIATION,
            })

print(json.dumps(findings, indent=2))
PYEOF
