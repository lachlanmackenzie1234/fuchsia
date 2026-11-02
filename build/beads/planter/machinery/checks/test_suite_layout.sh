#!/usr/bin/env bash
# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

set -euo pipefail

# Checks where the change hooks Bazel tests into the GN test graph, per
# docs/development/build/bazel_concepts/tests.md; only additions since
# $PLANTER_CHANGE_BASE count:
# 1. per_package_bazel_test_suite: a BUILD.gn gets a new bazel_test_suite()
#    listing individual tests while an ancestor area test_suite() of the same
#    kind already covers the directory.
# 2. bazel_test_suite_only_build_gn: a leaf BUILD.gn ends up holding only
#    imports, bazel_test_suite()s, and a group("tests") of them while an
#    ancestor area suite or non-umbrella ancestor BUILD.gn group("tests")
#    boundary exists.
# 3. nested_test_suite: a Bazel test_suite(tests) or GN bazel_test_suite()
#    gains a cross-package test_suite() entry (same-file sub-suites are fine).
# 4. implicit_test_suite: a newly added Bazel test_suite() omits an explicit
#    non-empty tests list.

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
target_dirs = [
    d.strip().strip("/")
    for d in re.split(r"[\s,]+", raw_dirs)
    if d.strip().strip("/")
]

UMBRELLA_ANCESTORS = {"", "src", "sdk", "build", "third_party", "zircon"}
SENTINEL_RE = re.compile(
    r"^##\s*BAZEL2GN SENTINEL|#LOCAL_BAZEL_BUILD_SENTINEL", re.M
)
SUITE_RE = re.compile(r'\bbazel_test_suite\(\s*"([^"]+)"\s*\)\s*\{')
SUITE_LIST_RE = re.compile(r"\b(host_tests|target_tests)\s*\+?=\s*\[([^\]]*)\]")

findings = []


# --- Git and diff-base helpers ---


def git_out(args):
    try:
        return subprocess.check_output(
            ["git", "-C", workdir] + args,
            stderr=subprocess.DEVNULL,
            text=True,
        ).strip()
    except Exception:
        return ""


def git_lines(args):
    out = git_out(args)
    return [line.strip() for line in out.splitlines() if line.strip()]


def is_ancestor(anc, desc):
    if not anc or not desc:
        return False
    return (
        subprocess.run(
            ["git", "-C", workdir, "merge-base", "--is-ancestor", anc, desc],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        ).returncode
        == 0
    )


def resolve_effective_diff_base():
    head_sha = git_out(["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])
    parent_sha = git_out(["rev-parse", "--verify", "--quiet", "HEAD~1^{commit}"])
    origin_main = git_out(
        ["rev-parse", "--verify", "--quiet", "origin/main^{commit}"]
    )
    base_sha = git_out(
        ["rev-parse", "--verify", "--quiet", f"{change_base}^{{commit}}"]
    )
    merge_base = (
        git_out(["merge-base", origin_main, head_sha])
        if (origin_main and head_sha)
        else ""
    )
    head_on_origin = bool(
        origin_main and head_sha and is_ancestor(head_sha, origin_main)
    )
    wt_files = git_lines(["diff", "--name-only", "HEAD"]) + git_lines(
        ["ls-files", "--others", "--exclude-standard"]
    )
    head_files = (
        git_lines(["diff", "--name-only", "HEAD~1", "HEAD"])
        if parent_sha
        else []
    )
    head_touches_target = bool(target_dirs) and any(
        any(p.startswith(td + "/") for td in target_dirs) for p in head_files
    )
    if (
        base_sha
        and merge_base
        and base_sha != merge_base
        and is_ancestor(base_sha, merge_base)
    ):
        return merge_base
    if parent_sha and (
        not base_sha
        or (
            change_base == "HEAD"
            and base_sha == head_sha
            and not head_on_origin
            and not wt_files
            and head_touches_target
        )
        or not is_ancestor(base_sha, head_sha)
    ):
        return merge_base or "HEAD~1"
    return change_base


effective_base = resolve_effective_diff_base()


def read_worktree_file(rel_path):
    try:
        with open(os.path.join(workdir, rel_path), "r", encoding="utf-8") as f:
            return f.read()
    except OSError:
        return ""


def read_base_file(rel_path):
    refs = [effective_base]
    if effective_base == "HEAD":
        refs = ["HEAD~1", "HEAD"]
    for ref in refs:
        if not ref:
            continue
        try:
            return subprocess.check_output(
                ["git", "-C", workdir, "show", f"{ref}:{rel_path}"],
                stderr=subprocess.DEVNULL,
                text=True,
            )
        except Exception:
            continue
    return ""


# --- GN and Bazel parsing helpers ---


def gn_code(text):
    """Blank out GN comments outside string literals while preserving line offsets."""
    return re.sub(
        r'("(?:\\.|[^"\\])*")|#[^\n]*',
        lambda m: m.group(1) or " " * len(m.group(0)),
        text,
    )


def closing_brace(code, start_idx):
    depth, in_str = 0, False
    i = start_idx
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


def split_label(label, current_pkg):
    label = re.sub(r"^@@?//", "//", label.strip())
    if label.startswith("@"):
        return None
    if label.startswith("//"):
        path, _, name = label[2:].partition(":")
        path = path.strip("/")
        return path, name or os.path.basename(path)
    return current_pkg, label.lstrip(":")


def list_strings(node):
    if isinstance(node, ast.BinOp) and isinstance(node.op, ast.Add):
        return list_strings(node.left) + list_strings(node.right)
    if isinstance(node, (ast.List, ast.Tuple)):
        return [
            e.value
            for e in node.elts
            if isinstance(e, ast.Constant) and isinstance(e.value, str)
        ]
    return []


def parse_bazel_suites_from_src(src_text, rel_path=""):
    """Return {suite_name: (lineno, tests_list_or_None)} for test_suite() calls."""
    suites = {}
    if not src_text.strip():
        return suites
    try:
        tree = ast.parse(src_text, filename=rel_path or "<BUILD.bazel>")
    except Exception:
        return suites
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call):
            continue
        fn = (
            node.func.id
            if isinstance(node.func, ast.Name)
            else (
                node.func.attr
                if isinstance(node.func, ast.Attribute)
                else ""
            )
        )
        if fn != "test_suite":
            continue
        kws = {kw.arg: kw.value for kw in node.keywords if kw.arg}
        name_node = kws.get("name")
        if not (
            isinstance(name_node, ast.Constant)
            and isinstance(name_node.value, str)
        ):
            continue
        lineno = getattr(node, "lineno", 1)
        tests_node = kws.get("tests")
        tests_val = None if tests_node is None else list_strings(tests_node)
        suites[name_node.value] = (lineno, tests_val)
    return suites


_bazel_suites_cache = {}
_base_bazel_suites_cache = {}


def bazel_suites_of(pkg):
    if pkg not in _bazel_suites_cache:
        rel = os.path.join(pkg, "BUILD.bazel") if pkg else "BUILD.bazel"
        _bazel_suites_cache[pkg] = parse_bazel_suites_from_src(
            read_worktree_file(rel), rel
        )
    return _bazel_suites_cache[pkg]


def base_bazel_suites_of(pkg):
    if pkg not in _base_bazel_suites_cache:
        rel = os.path.join(pkg, "BUILD.bazel") if pkg else "BUILD.bazel"
        _base_bazel_suites_cache[pkg] = parse_bazel_suites_from_src(
            read_base_file(rel), rel
        )
    return _base_bazel_suites_cache[pkg]


def parse_gn_bazel_test_suites(gn_text):
    """Return {suite_name: (lineno, {param: [labels]})} for bazel_test_suite() blocks."""
    code = gn_code(gn_text)
    out = {}
    for m in SUITE_RE.finditer(code):
        s_name = m.group(1)
        lineno = code.count("\n", 0, m.start()) + 1
        body = code[m.end() : closing_brace(code, m.end() - 1)]
        params = {}
        for lm in SUITE_LIST_RE.finditer(body):
            param = lm.group(1)
            lbls = re.findall(r'"([^"]+)"', lm.group(2))
            params.setdefault(param, []).extend(lbls)
        out[s_name] = (lineno, params)
    return out


# --- Ancestor test_suite and group("tests") discovery ---


def has_sibling_local_bazel_test_suite(td, anc_pkg, member_pkgs=()):
    cur = td
    while True:
        parent_dir = os.path.dirname(cur)
        if (
            not parent_dir
            or parent_dir == anc_pkg
            or (anc_pkg and not parent_dir.startswith(anc_pkg + "/"))
        ):
            break
        if any(
            p == parent_dir or p.startswith(parent_dir + "/")
            for p in member_pkgs
        ):
            cur = parent_dir
            continue
        parent_abs = os.path.join(workdir, parent_dir)
        cur_base = os.path.basename(cur)
        if os.path.isdir(parent_abs):
            try:
                entries = sorted(os.listdir(parent_abs))
            except OSError:
                entries = []
            for name in entries:
                if name == cur_base or name.startswith("."):
                    continue
                sib_gn = os.path.join(parent_dir, name, "BUILD.gn")
                if SUITE_RE.search(gn_code(read_worktree_file(sib_gn))):
                    return True
        cur = parent_dir
    return False


def ancestor_suite_covers_pkg(anc_pkg, anc_suite, td):
    if anc_pkg in UMBRELLA_ANCESTORS:
        return False
    cur = os.path.dirname(td)
    while cur and cur != anc_pkg and cur.startswith(anc_pkg + "/"):
        if bazel_suites_of(cur) or SUITE_RE.search(
            gn_code(read_worktree_file(os.path.join(cur, "BUILD.gn")))
        ):
            return False
        cur = os.path.dirname(cur)
    suite_info = bazel_suites_of(anc_pkg).get(anc_suite)
    if not suite_info:
        return False
    _, raw_members = suite_info
    in_scope = set(target_dirs) | {td}
    base_src = read_base_file(os.path.join(anc_pkg, "BUILD.bazel"))
    member_pkgs = set()
    for lbl in raw_members or []:
        sp = split_label(lbl, anc_pkg)
        if not sp:
            continue
        m_pkg, _ = sp
        if m_pkg in in_scope and f"//{m_pkg}:" not in base_src:
            continue
        member_pkgs.add(m_pkg)
    if td in member_pkgs:
        return True
    if has_sibling_local_bazel_test_suite(td, anc_pkg, member_pkgs):
        return False
    if os.path.dirname(td) == anc_pkg:
        return True
    rel_td = os.path.relpath(td, anc_pkg)
    first_seg = rel_td.split("/")[0]
    sub_root = os.path.join(anc_pkg, first_seg)
    if any(p == sub_root or p.startswith(sub_root + "/") for p in member_pkgs):
        return True
    return bool(member_pkgs)


def find_covering_ancestor_suite(td, param):
    parts = [p for p in td.split("/") if p]
    ancestors = ["/".join(parts[:i]) for i in range(1, len(parts))]
    for anc in reversed(ancestors):
        if anc in UMBRELLA_ANCESTORS:
            continue
        anc_gn = gn_code(read_worktree_file(os.path.join(anc, "BUILD.gn")))
        if not anc_gn:
            continue
        for _, (_, params) in parse_gn_bazel_test_suites(anc_gn).items():
            for lbl in params.get(param, []):
                sp = split_label(lbl, anc)
                if (
                    sp
                    and sp[0] == anc
                    and sp[1] in bazel_suites_of(anc)
                    and ancestor_suite_covers_pkg(anc, sp[1], td)
                ):
                    return (anc, sp[1])
    return None


def find_non_umbrella_ancestor_tests_boundary(td):
    parts = [p for p in td.split("/") if p]
    ancestors = ["/".join(parts[:i]) for i in range(1, len(parts))]
    for anc in reversed(ancestors):
        if anc in UMBRELLA_ANCESTORS:
            continue
        anc_gn_rel = os.path.join(anc, "BUILD.gn")
        anc_gn = gn_code(read_worktree_file(anc_gn_rel))
        if not anc_gn:
            continue
        rel_from_anc = os.path.relpath(td, anc)
        for m in re.finditer(r'\bgroup\(\s*"tests"\s*\)\s*\{', anc_gn):
            body = anc_gn[m.end() : closing_brace(anc_gn, m.end() - 1)]
            pat = (
                r'"(?://'
                + re.escape(td)
                + r"|"
                + re.escape(rel_from_anc)
                + r')(?::tests)?(?:\([^"]*\))?"'
            )
            if re.search(pat, body):
                return anc
    return None


# --- Check 1: per_package_bazel_test_suite ---


def check_per_package_bazel_test_suite(gn_rel):
    pkg_dir = os.path.dirname(gn_rel).strip("/")
    if not pkg_dir or pkg_dir in UMBRELLA_ANCESTORS:
        return
    cur_suites = parse_gn_bazel_test_suites(read_worktree_file(gn_rel))
    if not cur_suites:
        return
    base_suites = parse_gn_bazel_test_suites(read_base_file(gn_rel))
    local_bz_suites = bazel_suites_of(pkg_dir)

    for s_name, (lineno, params) in cur_suites.items():
        base_params = base_suites.get(s_name, (0, {}))[1]
        for param, labels in params.items():
            old_labels = set(base_params.get(param, []))
            new_labels = [lbl for lbl in labels if lbl not in old_labels]
            if not new_labels:
                continue
            anc_info = find_covering_ancestor_suite(pkg_dir, param)
            if not anc_info:
                continue
            anc_pkg, anc_suite = anc_info
            for lbl in new_labels:
                sp = split_label(lbl, pkg_dir)
                if not sp or sp[0] != pkg_dir or sp[1] in local_bz_suites:
                    continue
                t_name = sp[1]
                msg = (
                    f"`bazel_test_suite(\"{s_name}\")` in '{gn_rel}' lists individual "
                    f"test `\"//{pkg_dir}:{t_name}\"` while ancestor area `test_suite` "
                    f"`//{anc_pkg}:{anc_suite}` in '{anc_pkg}/BUILD.bazel' already "
                    f"covers `{pkg_dir}`."
                )
                rem = (
                    f"Wire `\"//{pkg_dir}:{t_name}\"` into "
                    f"`test_suite(name = \"{anc_suite}\", tests = [...])` in "
                    f"'{anc_pkg}/BUILD.bazel', set `visibility = "
                    f"[\"//{anc_pkg}:__pkg__\"]` on `:{t_name}` in "
                    f"'{pkg_dir}/BUILD.bazel', and remove "
                    f"`bazel_test_suite(\"{s_name}\")` from '{gn_rel}'."
                )
                findings.append({
                    "source": "test_suite_layout",
                    "category": "per_package_bazel_test_suite",
                    "severity": "error",
                    "file": gn_rel,
                    "line": lineno,
                    "message": msg,
                    "remediation": rem,
                })


# --- Check 2: bazel_test_suite_only_build_gn ---


def is_suite_only_build_gn(gn_text):
    if not gn_text.strip() or SENTINEL_RE.search(gn_text):
        return False
    code = gn_code(gn_text)
    suite_names = set()
    pos = 0
    n = len(code)
    while pos < n:
        while pos < n and code[pos].isspace():
            pos += 1
        if pos >= n:
            break
        m_imp = re.match(r'import\(\s*"[^"]+"\s*\)', code[pos:])
        if m_imp:
            pos += m_imp.end()
            continue
        m_call = re.match(
            r'([A-Za-z_]\w*)\(\s*"([^"]+)"\s*\)\s*\{', code[pos:]
        )
        if not m_call:
            return False
        tmpl, t_name = m_call.group(1), m_call.group(2)
        brace_open = pos + m_call.end() - 1
        brace_close = closing_brace(code, brace_open)
        body = code[brace_open + 1 : brace_close]
        if tmpl == "bazel_test_suite":
            suite_names.add(t_name)
        elif tmpl == "group" and t_name == "tests":
            deps = re.findall(r'"([^"]+)"', body)
            if not deps or not all(
                d.startswith(":") and d[1:] in suite_names for d in deps
            ):
                return False
        else:
            return False
        pos = brace_close + 1
    return bool(suite_names)


def check_bazel_test_suite_only_build_gn(gn_rel):
    pkg_dir = os.path.dirname(gn_rel).strip("/")
    if not pkg_dir or pkg_dir in UMBRELLA_ANCESTORS:
        return
    cur_text = read_worktree_file(gn_rel)
    if not is_suite_only_build_gn(cur_text):
        return
    base_text = read_base_file(gn_rel)
    if base_text and is_suite_only_build_gn(base_text):
        return
    # Keeping BUILD.gn is permitted when this package aggregates child subpackages.
    for _, members in bazel_suites_of(pkg_dir).values():
        for lbl in members or []:
            sp = split_label(lbl, pkg_dir)
            if sp and sp[0].startswith(pkg_dir + "/"):
                return
    cur_suites = parse_gn_bazel_test_suites(cur_text)
    anc_suite = None
    for _, (_, params) in cur_suites.items():
        for param in params:
            anc_suite = find_covering_ancestor_suite(pkg_dir, param)
            if anc_suite:
                break
        if anc_suite:
            break
    anc_boundary = find_non_umbrella_ancestor_tests_boundary(pkg_dir)
    if not anc_suite and not anc_boundary:
        return
    lineno = next(iter(cur_suites.values()))[0]
    if anc_suite:
        anc_pkg, anc_s_name = anc_suite
        rem = (
            f"Wire the Bazel test target(s) into `//{anc_pkg}:{anc_s_name}` in "
            f"'{anc_pkg}/BUILD.bazel', remove `{pkg_dir}:tests` from "
            f"'{anc_pkg}/BUILD.gn', and delete '{gn_rel}' (`git rm {gn_rel}`)."
        )
    else:
        rem = (
            f"Export the Bazel test target(s) at the ancestor `group(\"tests\")` "
            f"boundary in '{anc_boundary}/BUILD.gn' (or an ancestor `test_suite`), "
            f"remove `{pkg_dir}:tests` from '{anc_boundary}/BUILD.gn', and delete "
            f"'{gn_rel}' (`git rm {gn_rel}`)."
        )
    msg = (
        f"Leaf '{gn_rel}' holds only `import(...)`, `bazel_test_suite()`, and "
        f"`group(\"tests\")` while an ancestor test boundary already covers "
        f"`{pkg_dir}`."
    )
    findings.append({
        "source": "test_suite_layout",
        "category": "bazel_test_suite_only_build_gn",
        "severity": "error",
        "file": gn_rel,
        "line": lineno,
        "message": msg,
        "remediation": rem,
    })


# --- Check 3: nested_test_suite ---


def check_nested_test_suite_in_bazel(bz_rel):
    pkg_dir = os.path.dirname(bz_rel).strip("/")
    cur_suites = parse_bazel_suites_from_src(read_worktree_file(bz_rel), bz_rel)
    if not cur_suites:
        return
    base_suites = base_bazel_suites_of(pkg_dir)
    for s_name, (lineno, members) in cur_suites.items():
        if not members:
            continue
        old_members = set((base_suites.get(s_name) or (0, None))[1] or [])
        for lbl in members:
            if lbl in old_members:
                continue
            sp = split_label(lbl, pkg_dir)
            if not sp:
                continue
            m_pkg, m_name = sp
            if m_pkg != pkg_dir and m_name in bazel_suites_of(m_pkg):
                msg = (
                    f"`test_suite(name = \"{s_name}\")` in '{bz_rel}' references "
                    f"cross-package `test_suite` target `\"//{m_pkg}:{m_name}\"`; "
                    "area `test_suite` targets must list leaf test targets directly."
                )
                rem = (
                    f"Replace `\"//{m_pkg}:{m_name}\"` in `test_suite(name = "
                    f"\"{s_name}\")` in '{bz_rel}' with the individual test "
                    f"target label(s) from `//{m_pkg}`, and delete the nested "
                    f"`test_suite` `{m_name}` in '{m_pkg}/BUILD.bazel' if unused."
                )
                findings.append({
                    "source": "test_suite_layout",
                    "category": "nested_test_suite",
                    "severity": "error",
                    "file": bz_rel,
                    "line": lineno,
                    "message": msg,
                    "remediation": rem,
                })


def check_nested_test_suite_in_gn(gn_rel):
    pkg_dir = os.path.dirname(gn_rel).strip("/")
    cur_suites = parse_gn_bazel_test_suites(read_worktree_file(gn_rel))
    if not cur_suites:
        return
    base_suites = parse_gn_bazel_test_suites(read_base_file(gn_rel))
    for s_name, (lineno, params) in cur_suites.items():
        base_params = base_suites.get(s_name, (0, {}))[1]
        for param, labels in params.items():
            old_labels = set(base_params.get(param, []))
            for lbl in labels:
                if lbl in old_labels:
                    continue
                sp = split_label(lbl, pkg_dir)
                if not sp:
                    continue
                m_pkg, m_name = sp
                if m_pkg != pkg_dir and m_name in bazel_suites_of(m_pkg):
                    msg = (
                        f"`bazel_test_suite(\"{s_name}\")` in '{gn_rel}' references "
                        f"cross-package `test_suite` `\"//{m_pkg}:{m_name}\"` in "
                        f"`{param}`."
                    )
                    rem = (
                        f"Define a flat `test_suite(name = \"{param}\", tests = [...])` "
                        f"in '{pkg_dir}/BUILD.bazel' listing leaf test targets "
                        f"directly and reference `\"//{pkg_dir}:{param}\"` from "
                        f"`bazel_test_suite(\"{s_name}\")` in '{gn_rel}'."
                    )
                    findings.append({
                        "source": "test_suite_layout",
                        "category": "nested_test_suite",
                        "severity": "error",
                        "file": gn_rel,
                        "line": lineno,
                        "message": msg,
                        "remediation": rem,
                    })


# --- Check 4: implicit_test_suite ---


def check_implicit_test_suite(bz_rel):
    pkg_dir = os.path.dirname(bz_rel).strip("/")
    cur_suites = parse_bazel_suites_from_src(read_worktree_file(bz_rel), bz_rel)
    if not cur_suites:
        return
    base_suites = base_bazel_suites_of(pkg_dir)
    for s_name, (lineno, members) in cur_suites.items():
        if s_name in base_suites:
            old_members = base_suites[s_name][1]
            if not old_members:
                continue
        if members is None or len(members) == 0:
            detail = (
                "omits the `tests` attribute"
                if members is None
                else "sets an empty `tests = []` list"
            )
            msg = (
                f"`test_suite(name = \"{s_name}\")` in '{bz_rel}' {detail}, "
                "which implicitly expands to every test target in the package."
            )
            rem = (
                f"Pass an explicit non-empty `tests = [\"...\"]` list to "
                f"`test_suite(name = \"{s_name}\")` in '{bz_rel}'."
            )
            findings.append({
                "source": "test_suite_layout",
                "category": "implicit_test_suite",
                "severity": "error",
                "file": bz_rel,
                "line": lineno,
                "message": msg,
                "remediation": rem,
            })


# --- Main execution over changed BUILD.gn and BUILD.bazel files ---


changed = set(git_lines(["diff", "--name-only", effective_base]))
changed.update(git_lines(["diff", "--name-only", "HEAD"]))
changed.update(git_lines(["ls-files", "--others", "--exclude-standard"]))
for td in target_dirs:
    for bname in ("BUILD.bazel", "BUILD.gn"):
        rel = os.path.join(td, bname)
        if os.path.isfile(os.path.join(workdir, rel)):
            changed.add(rel)

changed_gn = sorted(
    f
    for f in changed
    if os.path.basename(f) == "BUILD.gn"
    and os.path.isfile(os.path.join(workdir, f))
)
changed_bz = sorted(
    f
    for f in changed
    if os.path.basename(f) in ("BUILD.bazel", "BUILD")
    and os.path.isfile(os.path.join(workdir, f))
)

for gn_rel in changed_gn:
    check_per_package_bazel_test_suite(gn_rel)
    check_bazel_test_suite_only_build_gn(gn_rel)
    check_nested_test_suite_in_gn(gn_rel)

for bz_rel in changed_bz:
    check_nested_test_suite_in_bazel(bz_rel)
    check_implicit_test_suite(bz_rel)

print(json.dumps(findings, indent=2))
PYEOF
