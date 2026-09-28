#!/usr/bin/env bash
# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

set -euo pipefail

# Deterministic check for GN-to-Bazel migration completeness and dual-build invariants:
# 1. skipped_convertible_target (error): flags any convertible library/binary/code target in
#    BUILD.bazel (e.g., fx_cc_library, cc_library, rustc_library, go_library, fidl_library)
#    annotated with `# @bazel2gn:skip` when BUILD.gn is present (especially reference or
#    bitrot-prevention libraries depended upon by `group("tests")` or duplicated above the
#    `## BAZEL2GN SENTINEL` marker in BUILD.gn).
# 2. duplicate_target_above_sentinel (error): flags any convertible target in BUILD.gn defined
#    above `## BAZEL2GN SENTINEL` when a target of the same name is defined in BUILD.bazel or
#    should be generated below the sentinel by `fx bazel2gn`.
# 3. missing_verify_bazel2gn_registration (error): flags any dual-build package whose BUILD.gn
#    defines `verify_bazel2gn("verify_bazel2gn")` without registering `//<pkg>:verify_bazel2gn`
#    in `//build/bazel2gn_verification_targets.gni` (or `//sdk/fidl/bazel2gn_verification_targets.gni`).

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

CONVERTIBLE_BAZEL_RULES = {
    "fx_cc_library",
    "cc_library",
    "fx_cc_library_headers",
    "cc_binary",
    "fx_cc_binary",
    "cc_shared_library_zx",
    "cc_source_library_zx",
    "cc_static_library_zx",
    "idk_cc_shared_library",
    "idk_cc_shared_library_zx",
    "idk_cc_source_library",
    "idk_cc_source_library_zx",
    "idk_cc_static_library",
    "idk_cc_static_library_zx",
    "rustc_library",
    "rust_library",
    "rustc_binary",
    "rust_binary",
    "rustc_proc_macro",
    "rust_proc_macro",
    "rustc_test",
    "go_library",
    "go_binary",
    "go_test",
    "py_library",
    "py_binary",
    "fidl_library",
    "zither_fidl_library",
}

CONVERTIBLE_GN_TEMPLATES = {
    "static_library",
    "source_set",
    "shared_library",
    "executable",
    "library_headers",
    "rustc_library",
    "rustc_binary",
    "rustc_macro",
    "rustc_test",
    "go_library",
    "go_binary",
    "go_test",
    "python_library",
    "python_binary",
    "fidl",
    "sdk_source_set",
    "sdk_static_library",
    "sdk_shared_library",
    "zx_library",
}


def git_lines(args):
    try:
        out = subprocess.check_output(
            ["git", "-C", workdir] + args, stderr=subprocess.DEVNULL, text=True
        )
    except Exception:
        return []
    return [l.strip() for l in out.splitlines() if l.strip()]


candidate_dirs = set()
if target_dir:
    candidate_dirs.add(target_dir)

changed = set()
# The task's change: everything since PLANTER_CHANGE_BASE ("HEAD~1" when HEAD is the task's own
# commit, "HEAD" when HEAD is unrelated upstream history) plus untracked files.
change_base = os.environ.get("PLANTER_CHANGE_BASE", "").strip() or "HEAD"
changed.update(git_lines(["diff", "--name-only", change_base]))
changed.update(git_lines(["ls-files", "--others", "--exclude-standard"]))
for p in changed:
    if os.path.basename(p) in ("BUILD.bazel", "BUILD.gn"):
        d = os.path.dirname(p).strip("/")
        if d and d != "build/bazel":
            candidate_dirs.add(d)


def parse_bazel_targets(bazel_path):
    try:
        text = open(bazel_path, encoding="utf-8").read()
    except Exception:
        return []
    lines = text.splitlines()
    targets = []
    try:
        tree = ast.parse(text, filename=bazel_path)
    except Exception:
        return []
    for node in tree.body:
        call = None
        if isinstance(node, ast.Expr) and isinstance(node.value, ast.Call):
            call = node.value
        if not call or not isinstance(call.func, ast.Name):
            continue
        rule = call.func.id
        tname = None
        for kw in call.keywords:
            if kw.arg == "name" and isinstance(kw.value, ast.Constant) and isinstance(kw.value.value, str):
                tname = kw.value.value
                break
        if not tname:
            continue
        start_line = node.lineno
        skip_line = None
        idx = start_line - 2
        while idx >= 0:
            s = lines[idx].strip()
            if not s:
                idx -= 1
                continue
            if s.startswith("#"):
                if "@bazel2gn:skip" in s:
                    skip_line = idx + 1
                idx -= 1
                continue
            break
        targets.append({
            "rule": rule,
            "name": tname,
            "line": start_line,
            "skip_line": skip_line,
        })
    return targets


def find_closing_brace(text, open_idx):
    depth, in_str, i, n = 0, False, open_idx, len(text)
    while i < n:
        c = text[i]
        if in_str:
            if c == "\\":
                i += 2
                continue
            if c == '"':
                in_str = False
        elif c == '"':
            in_str = True
        elif c == "#":
            nl = text.find("\n", i)
            i = n if nl < 0 else nl
            continue
        elif c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    return n


def parse_gn_file(gn_path):
    try:
        text = open(gn_path, encoding="utf-8").read()
    except Exception:
        return None
    sentinel_re = re.compile(r"^##\s*BAZEL2GN SENTINEL|#LOCAL_BAZEL_BUILD_SENTINEL", re.M)
    m = sentinel_re.search(text)
    has_sentinel = bool(m)
    has_verify = bool(re.search(r'\bverify_bazel2gn\(\s*"verify_bazel2gn"\s*\)', text))
    pre_text = text[: m.start()] if m else text
    post_text = text[m.end() :] if m else ""
    pre_targets = {}
    target_re = re.compile(r'^\s*([a-zA-Z0-9_]+)\(\s*"([^"]+)"\s*\)\s*\{', re.M)
    for tm in target_re.finditer(pre_text):
        tmpl, name = tm.group(1), tm.group(2)
        line_no = pre_text.count("\n", 0, tm.start()) + 1
        brace_idx = tm.end() - 1
        end_idx = find_closing_brace(pre_text, brace_idx)
        body = pre_text[brace_idx + 1 : end_idx]
        pre_targets[name] = (tmpl, line_no, body)
    return {
        "has_sentinel": has_sentinel,
        "has_verify": has_verify,
        "pre_text": pre_text,
        "post_text": post_text,
        "pre_targets": pre_targets,
    }


findings = []

for pkg_dir in sorted(candidate_dirs):
    bazel_abs = os.path.join(workdir, pkg_dir, "BUILD.bazel")
    gn_abs = os.path.join(workdir, pkg_dir, "BUILD.gn")
    bazel_rel = os.path.join(pkg_dir, "BUILD.bazel")
    gn_rel = os.path.join(pkg_dir, "BUILD.gn")

    if not os.path.isfile(bazel_abs):
        continue

    bazel_targets = parse_bazel_targets(bazel_abs)
    gn_info = parse_gn_file(gn_abs) if os.path.isfile(gn_abs) else None

    if gn_info and gn_info["has_sentinel"]:
        bazel_names = {t["name"] for t in bazel_targets}
        for t in bazel_targets:
            tname = t["name"]
            rule = t["rule"]
            in_pre_gn = (
                tname in gn_info["pre_targets"]
                and gn_info["pre_targets"][tname][0] in CONVERTIBLE_GN_TEMPLATES
            )
            ref_in_pre_gn = rule in CONVERTIBLE_BAZEL_RULES and bool(
                re.search(r'":' + re.escape(tname) + r'(?:"|\()', gn_info["pre_text"])
            )
            if t["skip_line"] is not None and (
                rule in CONVERTIBLE_BAZEL_RULES or in_pre_gn or ref_in_pre_gn
            ):
                findings.append({
                    "source": "migration_sanity",
                    "category": "skipped_convertible_target",
                    "severity": "error",
                    "file": bazel_rel,
                    "line": t["skip_line"],
                    "message": (
                        f"Target '{tname}' ({rule}) in '{bazel_rel}' is annotated with `# @bazel2gn:skip`"
                        + (f" and duplicated as `{gn_info['pre_targets'][tname][0]}(\"{tname}\")` above the BAZEL2GN SENTINEL in '{gn_rel}'" if in_pre_gn else "")
                        + (f" while referenced by `:{tname}` in '{gn_rel}'" if ref_in_pre_gn else "")
                        + ". Convertible libraries and reference/bitrot-prevention targets must not be skipped."
                    ),
                    "remediation": (
                        f"Remove `# @bazel2gn:skip` above `{rule}(name = \"{tname}\")` in '{bazel_rel}', "
                        f"delete any manual `{tname}` target definition above `## BAZEL2GN SENTINEL` in '{gn_rel}' "
                        "(keeping only helper `config(...)` or test package definitions above the sentinel), "
                        "use `# @bazel2gn:raw_overwrite:[ \"<gn_config_1>\", ... ]` on the closing `],` of `copts = [...]` "
                        "if C/C++ compiler flags need to map to GN `configs`, and re-run `fx bazel2gn -d "
                        + pkg_dir
                        + "`."
                    ),
                })

        for gname, (gtmpl, gline, gbody) in sorted(gn_info["pre_targets"].items()):
            if gtmpl in CONVERTIBLE_GN_TEMPLATES:
                unmigrated_deps = []
                declare_args_deps = []
                for dm in re.finditer(r'"//([^":(\s]+)(?::[^"(\s]+)?"', gbody):
                    dep_pkg = dm.group(1).strip("/")
                    if (
                        not dep_pkg
                        or dep_pkg == pkg_dir
                        or dep_pkg.startswith(("build/", "prebuilt/", "third_party/", "out/"))
                    ):
                        continue
                    dep_gn = os.path.join(workdir, dep_pkg, "BUILD.gn")
                    dep_bazel = os.path.join(workdir, dep_pkg, "BUILD.bazel")
                    if os.path.isfile(dep_gn) and not os.path.isfile(dep_bazel):
                        if f"//{dep_pkg}" not in unmigrated_deps:
                            unmigrated_deps.append(f"//{dep_pkg}")
                        try:
                            dep_gn_txt = open(dep_gn, encoding="utf-8").read()
                            if "declare_args(" in dep_gn_txt and f"//{dep_pkg}" not in declare_args_deps:
                                declare_args_deps.append(f"//{dep_pkg}")
                        except Exception:
                            pass

                attr_hints = []
                if gtmpl == "rustc_binary":
                    out_m = re.search(r'\b(?:output_name|name)\s*=\s*"([^"]+)"', gbody)
                    if out_m and out_m.group(1) != gname:
                        attr_hints.append(
                            f"set `crate_name = \"{out_m.group(1)}\"` on `rustc_binary(name = \"{gname}\")` in '{bazel_rel}' "
                            f"(bazel2gn maps `crate_name` on `rustc_binary` to `output_name = \"{out_m.group(1)}\"` in BUILD.gn; "
                            "omit dummy `version` strings)"
                        )
                    src_m = re.search(r'\bsource_root\s*=\s*"([^"]+)"', gbody)
                    if src_m and src_m.group(1) != "src/main.rs":
                        attr_hints.append(f"set `crate_root = \"{src_m.group(1)}\"`")
                if gtmpl.startswith("rustc_"):
                    cfg_m = re.search(r'\bconfigs\s*\+=\s*\[\s*"([^"]+)"\s*\]', gbody)
                    if cfg_m:
                        attr_hints.append(f"map `configs += [ \"{cfg_m.group(1)}\" ]` to `lint_config = \"{cfg_m.group(1)}\"`")

                dep_msg = (
                    f" Unmigrated first-party dependencies referenced by `{gname}` that also need a BUILD.bazel in this change: "
                    + ", ".join(unmigrated_deps)
                    + "."
                    if unmigrated_deps
                    else ""
                )
                dep_rem = ""
                if unmigrated_deps:
                    dep_rem = (
                        " First migrate unmigrated dependency package(s) "
                        + ", ".join(unmigrated_deps)
                        + " (create their BUILD.bazel, sync with `fx bazel2gn -d <dep_dir>`, and register `verify_bazel2gn`)."
                    )
                    if declare_args_deps:
                        dep_rem += (
                            " For `declare_args()` in "
                            + ", ".join(declare_args_deps)
                            + ", keep `declare_args() { ... }` in a `.gni` file (imported above `## BAZEL2GN SENTINEL` in its BUILD.gn), "
                            "export `<arg>` via `generated_file(\"gn_build_variables_for_bazel\")` in `//build/bazel/BUILD.gn` with "
                            "`# LINT.IfChange` / `# LINT.ThenChange(...)` on both sides, `load(\"@fuchsia_build_info//:args.bzl\", \"<arg>\")` "
                            "in its BUILD.bazel, and translate `if (<arg>) { features = [ ... ] }` as `crate_features = [\"...\"] if <arg> else []`."
                        )
                hint_rem = (" In '" + bazel_rel + "', " + "; ".join(attr_hints) + ".") if attr_hints else ""

                findings.append({
                    "source": "migration_sanity",
                    "category": "duplicate_target_above_sentinel",
                    "severity": "error",
                    "file": gn_rel,
                    "line": gline,
                    "message": (
                        f"Convertible GN target `{gtmpl}(\"{gname}\")` remains above `## BAZEL2GN SENTINEL` "
                        f"in '{gn_rel}'"
                        + (" (duplicating the target in BUILD.bazel)" if gname in bazel_names else "")
                        + ". All convertible library/binary targets (including benchmark binaries and reference/bitrot-prevention libraries) "
                        "must be migrated to BUILD.bazel and generated below the sentinel by `fx bazel2gn`."
                        + dep_msg
                    ),
                    "remediation": (
                        f"Define `{gname}` in '{bazel_rel}' without `# @bazel2gn:skip`, delete `{gtmpl}(\"{gname}\")` "
                        f"from above `## BAZEL2GN SENTINEL` in '{gn_rel}', and run `fx bazel2gn -d {pkg_dir}`."
                        + dep_rem
                        + hint_rem
                    ),
                })

        if gn_info["has_verify"]:
            verify_label = f'"{  "//" + pkg_dir + ":verify_bazel2gn"  }"'
            gni_rel = (
                "sdk/fidl/bazel2gn_verification_targets.gni"
                if pkg_dir.startswith("sdk/fidl/")
                else "build/bazel2gn_verification_targets.gni"
            )
            gni_abs = os.path.join(workdir, gni_rel)
            if os.path.isfile(gni_abs):
                gni_text = open(gni_abs, encoding="utf-8").read()
                if verify_label not in gni_text:
                    findings.append({
                        "source": "migration_sanity",
                        "category": "missing_verify_bazel2gn_registration",
                        "severity": "error",
                        "file": gni_rel,
                        "line": 1,
                        "message": (
                            f"Dual-build package '//{pkg_dir}' defines `verify_bazel2gn` in '{gn_rel}', "
                            f"but {verify_label} is not registered in '{gni_rel}'."
                        ),
                        "remediation": (
                            f"Add `  {verify_label},` in alphabetical order to `{gni_rel}` so CQ continuously "
                            f"verifies bazel2gn synchronization for `//{pkg_dir}`."
                        ),
                    })

print(json.dumps(findings, indent=2))
PYEOF
