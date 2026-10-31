#!/usr/bin/env bash
# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

set -euo pipefail

# Deterministic check for lint-driven source edits during GN-to-Bazel migrations.
#
# 1. lint_driven_source_edit (error): any non-build file changed in the migration
#    change (git diff $PLANTER_CHANGE_BASE) whose diff hunks look like a lint/warning
#    "fix" (removed .clone(), added #[allow]/#[expect]/NOLINT, unused-import
#    removal, `let _ =` insertion, etc.). Reports exact file:line evidence.
# 2. lint_config_semantics (warning): BUILD.bazel `lint_config = "<label>"` whose
#    label is not under //build/config/rust/lints and whose resolved
#    rust_lint_config target either cannot be resolved or omits the default
#    clippy/rustc lints from //build/config/rust/lints. In Bazel, lint_config
#    REPLACES the macro default lint set, while GN `configs += [...]` / GN
#    lint_config APPENDS to the defaults; an area rust_lint_config must therefore
#    include the default clippy and rustc lint sets.

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

ALLOWED_BASENAMES = {"BUILD.gn", "BUILD.bazel", "BUILD", "MODULE.bazel"}
ALLOWED_SUFFIXES = (".gni", ".bzl", ".bazelrc")


def git(args):
    try:
        return subprocess.check_output(
            ["git", "-C", workdir] + args, stderr=subprocess.DEVNULL, text=True
        )
    except Exception:
        return ""


def is_build_file(path):
    base = os.path.basename(path)
    return (
        base in ALLOWED_BASENAMES
        or base.endswith(ALLOWED_SUFFIXES)
        or base.startswith("WORKSPACE")
    )


LINT_PATTERNS = [
    ("removed", re.compile(r"\.clone\(\)"), "removed a .clone() call (clippy redundant_clone style fix)"),
    ("removed", re.compile(r"^\s*use\s+[\w:{}, *]+;\s*$"), "removed a `use` import (unused-import style fix)"),
    ("added", re.compile(r"#!?\[\s*(allow|expect)\s*\("), "added a lint suppression attribute"),
    ("added", re.compile(r"NOLINT|clippy::"), "added a lint suppression/annotation"),
    ("added", re.compile(r"^\s*let\s+_\s*="), "added `let _ =` (unused-result style fix)"),
]

findings = []


def _emit(findings, category, severity, file, line, message, remediation=""):
    findings.append({"source": "lint_config_parity", "category": category, "severity": severity, "file": file, "line": line, "message": message, "remediation": remediation})


# Resolve effective change base when HEAD is already the task's migration commit (fallback for
# runs outside planter, which exports PLANTER_RESOLVED_CHANGE_BASE).
def resolve_change_base(raw_base):
    base = (raw_base or "").strip() or "HEAD"
    head_subj = git(["log", "-1", "--format=%s", "HEAD"]).strip()
    if head_subj.startswith("[bazel_migration]"):
        head_sha = git(["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]).strip()
        parent_sha = git(["rev-parse", "--verify", "--quiet", "HEAD~1^{commit}"]).strip()
        base_sha = git(["rev-parse", "--verify", "--quiet", f"{base}^{{commit}}"]).strip()
        if parent_sha and (base == "HEAD" or base_sha == head_sha):
            return "HEAD~1"
    return base


# Collect the unified diff of the task's change since PLANTER_CHANGE_BASE ("HEAD~1" when HEAD is
# the task's own commit, "HEAD" when HEAD is unrelated upstream history).
change_base = (
    os.environ.get("PLANTER_RESOLVED_CHANGE_BASE", "").strip()
    or resolve_change_base(os.environ.get("PLANTER_CHANGE_BASE", ""))
)
env_changed = os.environ.get("PLANTER_CHANGED_FILES", "").strip()
if env_changed:
    non_build = [p.strip() for p in env_changed.splitlines() if p.strip() and not is_build_file(p.strip())]
    diff_texts = [git(["diff", "--unified=0", change_base, "--"] + non_build)] if non_build else []
else:
    diff_texts = [git(["diff", "--unified=0", change_base])]

# Check whether a Rust source file belongs to a migrated package whose BUILD.bazel
# applies `//build/config/rust/lints:clippy_warn_all` to a test target (in Bazel,
# `clippy_warn_all` bakes `_CLIPPY_WARN_PRODUCTION` in, whereas GN `rustc_test`
# omits `clippy_warn_production`).
def has_clippy_warn_all_on_rust_test(rs_path):
    if not rs_path.endswith(".rs"):
        return False
    pkg = os.path.dirname(rs_path).strip("/")
    while pkg:
        bzl_path = os.path.join(workdir, pkg, "BUILD.bazel")
        if os.path.isfile(bzl_path):
            try:
                text = open(bzl_path, encoding="utf-8").read()
            except OSError:
                return False
            has_warn_all = "//build/config/rust/lints:clippy_warn_all" in text
            has_test = any(
                tok in text
                for tok in (
                    "rustc_test(",
                    "host_rustc_test(",
                    "with_unit_tests",
                    "with_host_unit_tests",
                )
            )
            return has_warn_all and has_test
        pkg = os.path.dirname(pkg).strip("/")
    return False


seen = set()
seen_allowed_test_files = set()
for text in diff_texts:
    cur_file = None
    new_line = 0
    for raw in text.splitlines():
        if raw.startswith("+++ "):
            p = raw[4:].strip()
            cur_file = p[2:] if p.startswith("b/") else (None if p == "/dev/null" else p)
            continue
        if raw.startswith("--- "):
            continue
        m = re.match(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@", raw)
        if m:
            new_line = int(m.group(1))
            continue
        if not cur_file or is_build_file(cur_file):
            continue
        if raw.startswith("+"):
            kind, body, line = "added", raw[1:], new_line
            new_line += 1
        elif raw.startswith("-"):
            kind, body, line = "removed", raw[1:], new_line
        else:
            continue
        for pkind, rx, why in LINT_PATTERNS:
            if pkind == kind and rx.search(body):
                if has_clippy_warn_all_on_rust_test(cur_file):
                    if cur_file not in seen_allowed_test_files:
                        seen_allowed_test_files.add(cur_file)
                        _emit(
                            findings,
                            "allowed_test_clippy_warn_all_edit",
                            "INFO",
                            cur_file,
                            line,
                            (
                                f"Allowed clippy_warn_all test lint fix ({why}) in '{cur_file}' "
                                "to preserve clippy coverage without disable_clippy = True."
                            ),
                        )
                    continue
                key = (cur_file, line, why)
                if key in seen:
                    continue
                seen.add(key)
                _emit(
                    findings,
                    "lint_driven_source_edit",
                    "error",
                    cur_file,
                    line,
                    f"Migration {why} in non-build file '{cur_file}': `{body.strip()[:120]}`. "
                    "GN-to-Bazel migrations must not edit sources to silence lints/warnings.",
                    "Revert the source edit (git checkout $PLANTER_CHANGE_BASE -- <file>). "
                    "Pre-existing lint findings are out of scope. If the Bazel build newly reports this "
                    "lint, restore lint parity in BUILD.bazel (lint_config, rustc_flags, testonly, "
                    "features) instead; if impossible, stop and report the blocker.",
                )

# lint_config semantics check for BUILD.bazel files in the target package tree:
# resolve custom lint_config labels (including through alias() chains and loaded .bzl
# constants) and warn only when the target cannot be resolved to a rust_lint_config or
# when its clippy/rustc dicts omit the default lints from //build/config/rust/lints.
LINTS_PKG = "build/config/rust/lints"
BAZEL_BUILD_FILES = ("BUILD.bazel", "BUILD")


def read_rel(rel):
    try:
        with open(os.path.join(workdir, rel), encoding="utf-8") as f:
            return f.read()
    except Exception:
        return None


def parse_rel(rel):
    text = read_rel(rel)
    if text is None:
        return None
    try:
        return ast.parse(text, filename=rel)
    except Exception:
        return None


def resolve_label(label, cur_pkg):
    if not label or label.startswith("@"):
        return None
    if label.startswith(":"):
        return cur_pkg, label[1:]
    if label.startswith("//"):
        body = label[2:]
        if ":" in body:
            p, n = body.split(":", 1)
        else:
            p, n = body, body.rsplit("/", 1)[-1]
        return p.strip("/"), n
    return None


def eval_dict_branches(node, env):
    """Evaluates a Starlark dict / variable / BitOr / select() into a list of branch dicts."""
    if node is None:
        return None
    if isinstance(node, ast.Dict):
        d = {}
        for k, v in zip(node.keys, node.values):
            if (
                isinstance(k, ast.Constant)
                and isinstance(k.value, str)
                and isinstance(v, ast.Constant)
                and isinstance(v.value, str)
            ):
                d[k.value] = v.value
        return [d]
    if isinstance(node, ast.Name):
        return env.get(node.id)
    if isinstance(node, ast.BinOp) and isinstance(node.op, ast.BitOr):
        left = eval_dict_branches(node.left, env)
        right = eval_dict_branches(node.right, env)
        if not left or not right:
            return left or right
        return [{**dl, **dr} for dl in left for dr in right]
    if (
        isinstance(node, ast.Call)
        and isinstance(node.func, ast.Name)
        and node.func.id == "select"
        and node.args
        and isinstance(node.args[0], ast.Dict)
    ):
        branches = []
        for val_node in node.args[0].values:
            sub = eval_dict_branches(val_node, env)
            if sub:
                branches.extend(sub)
        return branches or None
    return None


_bzl_cache = {}
_pkg_cache = {}


def eval_bzl_file(rel, depth=0):
    if rel in _bzl_cache:
        return _bzl_cache[rel]
    if depth > 5:
        return {}
    tree = parse_rel(rel)
    if tree is None:
        _bzl_cache[rel] = {}
        return {}
    cur_pkg = os.path.dirname(rel)
    env = {}
    for node in tree.body:
        if (
            isinstance(node, ast.Expr)
            and isinstance(node.value, ast.Call)
            and isinstance(node.value.func, ast.Name)
            and node.value.func.id == "load"
            and node.value.args
            and isinstance(node.value.args[0], ast.Constant)
            and isinstance(node.value.args[0].value, str)
        ):
            resolved = resolve_label(node.value.args[0].value, cur_pkg)
            if resolved:
                bpkg, bfile = resolved
                brel = os.path.join(bpkg, bfile) if bpkg else bfile
                benv = eval_bzl_file(brel, depth + 1)
                for arg in node.value.args[1:]:
                    if isinstance(arg, ast.Constant) and isinstance(arg.value, str) and arg.value in benv:
                        env[arg.value] = benv[arg.value]
                for kw in node.value.keywords:
                    if (
                        kw.arg
                        and isinstance(kw.value, ast.Constant)
                        and isinstance(kw.value.value, str)
                        and kw.value.value in benv
                    ):
                        env[kw.arg] = benv[kw.value.value]
        elif isinstance(node, ast.Assign) and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name):
            branches = eval_dict_branches(node.value, env)
            if branches is not None:
                env[node.targets[0].id] = branches
    _bzl_cache[rel] = env
    return env


def eval_pkg_targets(pkg):
    if pkg in _pkg_cache:
        return _pkg_cache[pkg]
    tree = None
    for base in BAZEL_BUILD_FILES:
        rel = os.path.join(pkg, base) if pkg else base
        tree = parse_rel(rel)
        if tree is not None:
            break
    if tree is None:
        _pkg_cache[pkg] = {}
        return {}
    env = {}
    out = {}
    for node in tree.body:
        if (
            isinstance(node, ast.Expr)
            and isinstance(node.value, ast.Call)
            and isinstance(node.value.func, ast.Name)
            and node.value.func.id == "load"
            and node.value.args
            and isinstance(node.value.args[0], ast.Constant)
            and isinstance(node.value.args[0].value, str)
        ):
            resolved = resolve_label(node.value.args[0].value, pkg)
            if resolved:
                bpkg, bfile = resolved
                brel = os.path.join(bpkg, bfile) if bpkg else bfile
                benv = eval_bzl_file(brel)
                for arg in node.value.args[1:]:
                    if isinstance(arg, ast.Constant) and isinstance(arg.value, str) and arg.value in benv:
                        env[arg.value] = benv[arg.value]
                for kw in node.value.keywords:
                    if (
                        kw.arg
                        and isinstance(kw.value, ast.Constant)
                        and isinstance(kw.value.value, str)
                        and kw.value.value in benv
                    ):
                        env[kw.arg] = benv[kw.value.value]
        elif isinstance(node, ast.Assign) and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name):
            branches = eval_dict_branches(node.value, env)
            if branches is not None:
                env[node.targets[0].id] = branches
        elif isinstance(node, ast.Expr) and isinstance(node.value, ast.Call) and isinstance(node.value.func, ast.Name):
            call = node.value
            rule = call.func.id
            tname = None
            actual = None
            clippy_branches = None
            rustc_branches = None
            for kw in call.keywords:
                if kw.arg == "name" and isinstance(kw.value, ast.Constant) and isinstance(kw.value.value, str):
                    tname = kw.value.value
                elif kw.arg == "actual" and isinstance(kw.value, ast.Constant) and isinstance(kw.value.value, str):
                    actual = kw.value.value
                elif kw.arg == "clippy":
                    clippy_branches = eval_dict_branches(kw.value, env)
                elif kw.arg == "rustc":
                    rustc_branches = eval_dict_branches(kw.value, env)
            if tname:
                out[tname] = {
                    "rule": rule,
                    "actual": actual,
                    "clippy": clippy_branches,
                    "rustc": rustc_branches,
                }
    _pkg_cache[pkg] = out
    return out


def resolve_lint_config_target(pkg, name, depth=0):
    if depth > 5:
        return None
    t = eval_pkg_targets(pkg).get(name)
    if not t:
        return None
    if t["rule"] == "alias" and t["actual"]:
        r = resolve_label(t["actual"], pkg)
        if r:
            return resolve_lint_config_target(r[0], r[1], depth + 1)
    return pkg, name, t


def extract_label_strings(node):
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        yield node.value, getattr(node, "lineno", 1)
    elif isinstance(node, ast.Dict):
        for v in node.values:
            yield from extract_label_strings(v)
    elif isinstance(node, ast.Call):
        for a in node.args:
            yield from extract_label_strings(a)
    elif isinstance(node, ast.BinOp):
        yield from extract_label_strings(node.left)
        yield from extract_label_strings(node.right)


default_lints_targets = eval_pkg_targets(LINTS_PKG)
default_clippy_dict = (
    (default_lints_targets.get("clippy_warn_default", {}).get("clippy") or [{}])[0]
)
production_clippy_dict = (
    (default_lints_targets.get("clippy_warn_production", {}).get("clippy") or [{}])[0]
)
default_rustc_dict = (
    (default_lints_targets.get("clippy_warn_default", {}).get("rustc") or [{}])[0]
)

# Check whether a package's BUILD.gn restricts the entire file to the host toolchain via a
# top-level `assert(is_host, ...)`, in which case skipping `target_compatible_with =
# HOST_OS_CONSTRAINTS` in bazel2gn avoids wrapping generated targets in a redundant
# `if (is_host)` block that alters shac test-parity polarity on unguarded pre-migration targets.
def has_file_scope_assert_is_host(pkg):
    gn_rel = os.path.join(pkg, "BUILD.gn") if pkg else "BUILD.gn"
    gn_text = read_rel(gn_rel) or git(["show", f"{change_base}:{gn_rel}"]) or ""
    pre_sentinel = gn_text.split("## BAZEL2GN SENTINEL", 1)[0]
    return bool(re.search(r"(?m)^\s*assert\(\s*is_host\b", pre_sentinel))


# Strip `#` comments and top-level `template(...) { ... }` / `if (...is_host...) { ... }` blocks
# from a GN file so we can find unguarded top-level `rustc_*` targets with `with_unit_tests = true`.
def find_unguarded_gn_rust_unit_tests(gn_text):
    lines = [
        re.sub(r"#.*$", "", ln)
        for ln in gn_text.split("## BAZEL2GN SENTINEL", 1)[0].splitlines()
    ]
    cleaned = "\n".join(lines)
    # Remove top-level `template(...) { ... }` and `if (...is_host...) { ... }` blocks.
    out = []
    i = 0
    n = len(cleaned)
    block_hdr = re.compile(
        r"\b(?:template\s*\([^)]*\)|if\s*\([^)]*\b(?:is_host|host_toolchain)\b[^)]*\))\s*\{"
    )
    while i < n:
        m = block_hdr.search(cleaned, i)
        if not m:
            out.append(cleaned[i:])
            break
        out.append(cleaned[i : m.start()])
        depth = 1
        j = m.end()
        while j < n and depth > 0:
            if cleaned[j] == "{":
                depth += 1
            elif cleaned[j] == "}":
                depth -= 1
            j += 1
        i = j
    top_gn = "".join(out)
    unit_tests = {}
    tgt_pat = re.compile(
        r'\b(rustc_(?:library|binary|proc_macro))\s*\(\s*"([^"]+)"\s*\)\s*\{'
    )
    for m in tgt_pat.finditer(top_gn):
        rule_type, tname = m.group(1), m.group(2)
        depth = 1
        j = m.end()
        while j < len(top_gn) and depth > 0:
            if top_gn[j] == "{":
                depth += 1
            elif top_gn[j] == "}":
                depth -= 1
            j += 1
        body = top_gn[m.end() : max(m.end(), j - 1)]
        if re.search(r"\bwith_unit_tests\s*=\s*true\b", body) and not re.search(
            r"\bwith_host_unit_tests\s*=\s*true\b", body
        ):
            unit_tests[f"{tname}_test"] = (rule_type, tname)
    return unit_tests


# Detect when a dual-build BUILD.gn with a file-scope `assert(is_host, ...)` had an unguarded
# top-level `rustc_*(with_unit_tests = true)` target at $PLANTER_CHANGE_BASE, and bazel2gn now
# wraps the generated `rustc_test("<name>_test")` in `if (is_host)` below the sentinel (which
# causes shac's `bazel_test_parity` check to fail with CLASS 1 SILENT ORPHANING on the device leg).
def check_host_assert_test_polarity_drift(cur_pkg, bzl_rel, tree):
    gn_rel = os.path.join(cur_pkg, "BUILD.gn") if cur_pkg else "BUILD.gn"
    after_gn = read_rel(gn_rel) or ""
    if "## BAZEL2GN SENTINEL" not in after_gn:
        return
    before_gn = git(["show", f"{change_base}:{gn_rel}"]) or ""
    if not before_gn or "## BAZEL2GN SENTINEL" in before_gn:
        return
    unguarded_tests = find_unguarded_gn_rust_unit_tests(before_gn)
    if not unguarded_tests:
        return
    post_gn = after_gn.split("## BAZEL2GN SENTINEL", 1)[1]
    bzl_lines = {}
    for stmt in tree.body:
        if isinstance(stmt, ast.Expr) and isinstance(stmt.value, ast.Call):
            for kw in stmt.value.keywords:
                if (
                    kw.arg == "name"
                    and isinstance(kw.value, ast.Constant)
                    and isinstance(kw.value.value, str)
                ):
                    bzl_lines[kw.value.value] = getattr(stmt, "lineno", 1)
    for test_name, (orig_rule, lib_name) in sorted(unguarded_tests.items()):
        # Check if `group("tests")` in before_gn referenced `:<test_name>` without `($host_toolchain)`.
        if not re.search(
            r'":' + re.escape(test_name) + r'"', before_gn
        ):
            continue
        # Check if `post_gn` wraps `rustc_test("<test_name>")` inside `if (is_host)`.
        if not re.search(
            r"\bif\s*\(\s*is_host\s*\)\s*\{\s*rustc_test\s*\(\s*\""
            + re.escape(test_name)
            + r"\"",
            post_gn,
        ):
            continue
        lineno = bzl_lines.get(test_name, 1)
        wrap_name = (
            f"{lib_name}_host_test"
            if not test_name.endswith("_host_test")
            else f"{test_name}_wrapper"
        )
        _emit(
            findings,
            "host_assert_test_polarity_drift",
            "error",
            bzl_rel,
            lineno,
            (
                f"Target '{test_name}' in {bzl_rel} generates `if (is_host) {{ "
                f'rustc_test("{test_name}") }}` below `## BAZEL2GN SENTINEL` in '
                f"{gn_rel}, but pre-migration {gn_rel} relied on a file-scope "
                f'`assert(is_host, ...)` with an unguarded `{orig_rule}("{lib_name}") '
                "{{ with_unit_tests = true }}`. Shac's `bazel_test_parity` check "
                "ignores file-scope `assert(is_host)` and classifies the "
                f"pre-migration `//{cur_pkg}:{test_name}` as a `device` leg, so "
                "wrapping the generated `rustc_test` in `if (is_host)` changes its "
                "inferred polarity to `host` and fails CQ with `CLASS 1 SILENT "
                "ORPHANING (REMOVED TEST TARGET WITH EXISTING SOURCES)`."
            ),
            (
                f"In {bzl_rel}, replace `host_rustc_test(name = \"{test_name}\", ...)` "
                f"with `rustc_test(name = \"{test_name}\", crate = \":{lib_name}\", ...)` "
                "(do not repeat `edition` when `crate` is set, and annotate "
                "`target_compatible_with = HOST_OS_CONSTRAINTS,  # @bazel2gn:skip` "
                "with an explanatory comment above it so `bazel2gn` omits the "
                "redundant `if (is_host)` block) plus "
                f"`wrap_host_rust_test(name = \"{wrap_name}\", binary_name = "
                f"\"{test_name}\", ...)` from "
                '`"//build/bazel/rules/host_tests:host_rustc_test.bzl"` (passing any '
                "runtime test data in `data` on `rustc_test` and `test_data` on "
                "`wrap_host_rust_test`), wire "
                f"`\"//{cur_pkg}:{wrap_name}\"` into the `host_tests` suite, and "
                f"re-run `fx bazel2gn -d {cur_pkg}`."
            ),
        )


if target_dir:
    root = os.path.join(workdir, target_dir)
    for dirpath, _dirs, files in os.walk(root):
        if "BUILD.bazel" not in files:
            continue
        path = os.path.join(dirpath, "BUILD.bazel")
        rel = os.path.relpath(path, workdir)
        cur_pkg = os.path.dirname(rel).strip("/")
        tree = parse_rel(rel)
        if tree is None:
            continue
        raw_bzl = read_rel(rel) or ""
        pkg_asserts_is_host = has_file_scope_assert_is_host(cur_pkg)
        if pkg_asserts_is_host:
            check_host_assert_test_polarity_drift(cur_pkg, rel, tree)
        for idx, bzl_line in enumerate(raw_bzl.splitlines(), start=1):
            if re.search(
                r"#\s*@bazel2gn:raw_attribute:.*\bexclude_toolchain_tags\s*\+?=",
                bzl_line,
            ):
                _emit(
                    findings,
                    "raw_attribute_for_supported_bazel2gn_tag",
                    "error",
                    rel,
                    idx,
                    (
                        f"Line {idx} in {rel} uses `# @bazel2gn:raw_attribute:` to emit "
                        "`exclude_toolchain_tags`, which `bazel2gn` supports natively."
                    ),
                    (
                        'Use `tags = ["exclude_toolchain_tags=<tag>", ...]` (plus '
                        "`target_compatible_with = HOST_OS_CONSTRAINTS` when host-only, "
                        "or `disable_instrumentation = True` on `fx_cc_*` for "
                        '`"instrumented"`) instead of `# @bazel2gn:raw_attribute:`. '
                        "If the binary is wrapped by `bazel_host_tool` in BUILD.gn "
                        "and marked `# @bazel2gn:skip`, remove the `raw_attribute`."
                    ),
                )
            if not pkg_asserts_is_host and re.search(
                r"\btarget_compatible_with\s*=\s*HOST_OS_CONSTRAINTS\b.*#\s*@bazel2gn:skip\b",
                bzl_line,
            ):
                base_gn_for_skip = git(["show", f"{change_base}:{cur_pkg}/BUILD.gn"])
                if not base_gn_for_skip and change_base == "HEAD":
                    base_gn_for_skip = git(["show", f"HEAD~1:{cur_pkg}/BUILD.gn"])
                base_gn_clean_skip = re.sub(r"#[^\n]*", "", base_gn_for_skip or "")
                if re.search(r"\bif\s*\(\s*is_host\b", base_gn_clean_skip):
                    _emit(
                        findings,
                        "unnecessary_skip_on_host_constraints",
                        "error",
                        rel,
                        idx,
                        (
                            f"Line {idx} in {rel} puts `# @bazel2gn:skip` on "
                            "`target_compatible_with = HOST_OS_CONSTRAINTS`."
                        ),
                        (
                            "Remove `# @bazel2gn:skip` from `target_compatible_with = "
                            "HOST_OS_CONSTRAINTS` (or use `host_rustc_test` / `host_go_test`); "
                            "`bazel2gn` natively translates it to `if (is_host)`."
                        ),
                    )
        # Check for dual-build packages where base BUILD.gn had an unguarded
        # `rustc_library` / `rustc_binary` / `rustc_proc_macro` with `with_unit_tests = true`
        # without `if (is_host)`, `assert(is_host)`, or a local `group("tests")` referencing
        # `($host_toolchain)`. In such packages, `scripts/shac/check_bazel_test_parity.py`
        # (`GN_RUST_UNITTEST_OWNER_RULES`) classifies the pre-migration unit test in `before_gn`
        # as `"device"` polarity (unlike `ffx_plugin`, which wraps its inner targets in
        # `if (is_host)` inside `ffx_plugin.gni` and is not in `GN_RUST_UNITTEST_OWNER_RULES`),
        # so if `bazel2gn` wraps the generated `rustc_test` in `if (is_host)` (from
        # `host_rustc_test` or unskipped `target_compatible_with = HOST_OS_CONSTRAINTS`),
        # `shac` fails with `CLASS 1 SILENT ORPHANING (REMOVED TEST TARGET WITH EXISTING SOURCES)`.
        base_gn_for_pol = git(["show", f"{change_base}:{cur_pkg}/BUILD.gn"])
        if (
            not base_gn_for_pol or "## BAZEL2GN SENTINEL" in base_gn_for_pol
        ) and change_base == "HEAD":
            cand_pol = git(["show", f"HEAD~1:{cur_pkg}/BUILD.gn"])
            if cand_pol:
                base_gn_for_pol = cand_pol
        base_gn_pol_clean = re.sub(r"#[^\n]*", "", base_gn_for_pol or "")
        unguarded_pol_tests = find_unguarded_gn_rust_unit_tests(
            base_gn_for_pol or ""
        )
        cur_gn_full = os.path.join(workdir, cur_pkg, "BUILD.gn")
        cur_gn_text = ""
        if os.path.isfile(cur_gn_full):
            try:
                with open(cur_gn_full, encoding="utf-8", errors="ignore") as gf:
                    cur_gn_text = gf.read()
            except OSError:
                pass
        if (
            "## BAZEL2GN SENTINEL" in cur_gn_text
            and "## BAZEL2GN SENTINEL" not in base_gn_pol_clean
            and unguarded_pol_tests
            and not re.search(
                r"\b(?:if|assert)\s*\(\s*is_host\b", base_gn_pol_clean
            )
            and "host_toolchain" not in base_gn_pol_clean
            and not re.search(
                r"\b(?:fuchsia_unittest_package|fuchsia_test_package)\s*\(",
                base_gn_pol_clean,
            )
        ):
            bzl_src_pol = ""
            try:
                with open(
                    os.path.join(workdir, rel),
                    encoding="utf-8",
                    errors="ignore",
                ) as bf_pol:
                    bzl_src_pol = bf_pol.read()
            except OSError:
                pass
            for node_p in ast.walk(tree):
                if not (
                    isinstance(node_p, ast.Call)
                    and isinstance(node_p.func, ast.Name)
                ):
                    continue
                r_p = node_p.func.id
                if r_p not in ("host_rustc_test", "rustc_test"):
                    continue
                seg_p = ast.get_source_segment(bzl_src_pol, node_p) or ""
                has_unskipped_host_tcw = bool(
                    re.search(
                        r"\btarget_compatible_with\s*=\s*HOST_OS_CONSTRAINTS\b"
                        r"(?![^\n]*#\s*@bazel2gn:skip\b)",
                        seg_p,
                    )
                )
                if r_p == "host_rustc_test" or has_unskipped_host_tcw:
                    t_p = "<test>"
                    for kw_p in node_p.keywords:
                        if (
                            kw_p.arg == "name"
                            and isinstance(kw_p.value, ast.Constant)
                            and isinstance(kw_p.value.value, str)
                        ):
                            t_p = kw_p.value.value
                    if t_p not in unguarded_pol_tests:
                        continue
                    orig_rule_p, lib_name_p = unguarded_pol_tests[t_p]
                    wrap_name_p = (
                        f"{lib_name_p}_host_test"
                        if not t_p.endswith("_host_test")
                        else f"{t_p}_wrapper"
                    )
                    _emit(
                        findings,
                        "shac_unconditional_gn_unit_test_polarity_shift",
                        "error",
                        rel,
                        getattr(node_p, "lineno", 1),
                        (
                            f"Target '{t_p}' ({r_p}) in '{rel}' causes `fx bazel2gn` to wrap "
                            f'`rustc_test("{t_p}")` in `if (is_host)` in '
                            f"'{cur_pkg}/BUILD.gn', whereas base '{cur_pkg}/BUILD.gn' declared "
                            f'`{orig_rule_p}("{lib_name_p}")` with `with_unit_tests = true` '
                            "without `if (is_host)`, `assert(is_host)`, or a local "
                            '`group("tests")` `$host_toolchain` edge. This causes '
                            "`scripts/shac/check_bazel_test_parity.py` to classify the base GN "
                            "test as `device` polarity and the generated GN test as `host` "
                            "polarity, failing `shac check` with `CLASS 1 SILENT ORPHANING`."
                        ),
                        (
                            f"In '{rel}', replace `host_rustc_test(name = \"{t_p}\", ...)` "
                            f"with `rustc_test(name = \"{t_p}\", crate = \":{lib_name_p}\", ...)` "
                            '(loaded from `"//build/bazel/rules/rust:rustc_test.bzl"`, omitting '
                            "`edition` when `crate` is set) and annotate "
                            "`target_compatible_with = HOST_OS_CONSTRAINTS,  # @bazel2gn:skip` "
                            "(with an explanatory comment line directly above "
                            "`target_compatible_with`) on both `rustc_test` and `rustc_library`, "
                            f'add `wrap_host_rust_test(name = "{wrap_name_p}", binary_name = '
                            f'"{t_p}", ...)` from '
                            '`"//build/bazel/rules/host_tests:host_rustc_test.bzl"` wired as '
                            f'`"//{cur_pkg}:{wrap_name_p}"` into the `host_tests` suite, and '
                            f"re-run `fx bazel2gn -d {cur_pkg}` so the generated GN targets in "
                            f"'{cur_pkg}/BUILD.gn' remain unguarded by `if (is_host)`."
                        ),
                    )
        for node in ast.walk(tree):
            if not isinstance(node, ast.Call):
                continue
            rule_name = node.func.id if isinstance(node.func, ast.Name) else ""
            is_testonly = False
            lint_kw = None
            disable_clippy_kw = None
            target_name = "<target>"
            for kw in node.keywords:
                if kw.arg == "name" and isinstance(kw.value, ast.Constant) and isinstance(kw.value.value, str):
                    target_name = kw.value.value
                elif kw.arg == "testonly" and isinstance(kw.value, ast.Constant) and kw.value.value is True:
                    is_testonly = True
                elif kw.arg == "lint_config":
                    lint_kw = kw
                elif (
                    kw.arg == "disable_clippy"
                    and isinstance(kw.value, ast.Constant)
                    and kw.value.value is True
                ):
                    disable_clippy_kw = kw
            if disable_clippy_kw is not None:
                base_gn = git(["show", f"{change_base}:{cur_pkg}/BUILD.gn"])
                if not re.search(r"\bdisable_clippy\s*=\s*true\b", base_gn):
                    _emit(
                        findings,
                        "unrequested_disable_clippy",
                        "error",
                        rel,
                        getattr(disable_clippy_kw, "lineno", getattr(node, "lineno", 1)),
                        (
                            f"Target '{target_name}' in {rel} sets `disable_clippy = True`, "
                            f"but `{cur_pkg}/BUILD.gn` did not set `disable_clippy = true` at "
                            "the change base. Disabling clippy loses lint coverage during migration."
                        ),
                        (
                            "Remove `disable_clippy = True` from BUILD.bazel. If `clippy_warn_all` "
                            "on a Rust test target in Bazel surfaces a `_CLIPPY_WARN_PRODUCTION` "
                            "lint (such as `clippy::redundant_clone`, which GN `rustc_test` "
                            "omitted), fix the lint or add a narrow `#[allow(clippy::<lint>)]` "
                            "in the test source file instead of disabling clippy on the target."
                        ),
                    )
            if lint_kw is None:
                continue
            is_test_target = is_testonly or rule_name in ("rustc_test", "rust_test")
            req_clippy = default_clippy_dict if is_test_target else production_clippy_dict
            for label, lineno in extract_label_strings(lint_kw.value):
                if label == "//conditions:default":
                    continue
                resolved = resolve_label(label, cur_pkg)
                if not resolved:
                    continue
                lpkg, lname = resolved
                if lpkg == LINTS_PKG:
                    continue
                target_res = resolve_lint_config_target(lpkg, lname)
                if not target_res or target_res[2]["rule"] != "rust_lint_config":
                    _emit(
                        findings,
                        "lint_config_semantics",
                        "warning",
                        rel,
                        lineno,
                        f'lint_config = "{label}" does not resolve to a `rust_lint_config` target. '
                        "Bazel lint_config REPLACES the macro default lint set while GN appends to it.",
                        "Point `lint_config` at a valid `rust_lint_config` target (or pre-existing alias) "
                        "that includes the default clippy and rustc lint sets from //build/config/rust/lints.",
                    )
                    continue
                def_pkg, def_name, tinfo = target_res
                clippy_branches = tinfo.get("clippy") or []
                rustc_branches = tinfo.get("rustc") or []
                missing_clippy = sorted({
                    k for k in req_clippy
                    if k != "all" and (not clippy_branches or any(k not in b for b in clippy_branches))
                })
                missing_rustc = sorted({
                    k for k in default_rustc_dict
                    if not rustc_branches or any(k not in b for b in rustc_branches)
                })
                if missing_clippy or missing_rustc:
                    missing_desc = []
                    if missing_clippy:
                        missing_desc.append(f"clippy lints ({', '.join(missing_clippy[:5])})")
                    if missing_rustc:
                        missing_desc.append(f"rustc lints ({', '.join(missing_rustc[:5])})")
                    _emit(
                        findings,
                        "lint_config_semantics",
                        "warning",
                        rel,
                        lineno,
                        f'lint_config = "{label}" (resolving to //{def_pkg}:{def_name}) omits default '
                        f"{' and '.join(missing_desc)} from //build/config/rust/lints. "
                        "Bazel lint_config REPLACES the macro default lint set while GN appends to it, "
                        "so omitting the defaults changes the effective lint set.",
                        f"Compose `//{def_pkg}:{def_name}` with the default clippy/rustc lint constants from "
                        "//build/config/rust/lints so the Bazel lint set matches GN's append semantics.",
                    )

    # Also run //scripts/shac/check_bazel_test_parity.py directly against change_base when present
    # (shac skips bazel_test_parity on uncommitted working trees when HEAD has Reviewed-on:).
    parity_script = os.path.join(workdir, "scripts", "shac", "check_bazel_test_parity.py")
    if os.path.isfile(parity_script):
        try:
            import importlib.util
            import pathlib

            spec = importlib.util.spec_from_file_location("check_bazel_test_parity", parity_script)
            if spec and spec.loader:
                mod = importlib.util.module_from_spec(spec)
                sys.modules["check_bazel_test_parity"] = mod
                spec.loader.exec_module(mod)
                drift_files = {
                    os.path.dirname(f["file"]).strip("/")
                    for f in findings
                    if f.get("category")
                    in (
                        "host_assert_test_polarity_drift",
                        "shac_unconditional_gn_unit_test_polarity_shift",
                    )
                }
                for pf in mod.analyze_repository(
                    pathlib.Path(workdir), base_ref=change_base
                ):
                    if getattr(pf, "level", "") != "error":
                        continue
                    fp = getattr(pf, "filepath", "") or ""
                    fp_pkg = os.path.dirname(fp).strip("/") if fp else ""
                    if fp and not (
                        fp == target_dir
                        or fp.startswith(target_dir + "/")
                        or fp_pkg == target_dir
                    ):
                        continue
                    msg = getattr(pf, "message", "") or ""
                    if fp_pkg in drift_files and "REMOVED TEST TARGET WITH EXISTING SOURCES" in msg:
                        continue
                    parts = msg.split("\nRemediation:", 1)
                    main_msg = parts[0].strip()
                    rem_msg = (
                        parts[1].strip()
                        if len(parts) > 1
                        else "Fix the Bazel/GN test parity error reported by check_bazel_test_parity.py."
                    )
                    _emit(
                        findings,
                        "shac_bazel_test_parity",
                        "error",
                        fp or f"{target_dir}/BUILD.bazel",
                        getattr(pf, "line", 1) or 1,
                        main_msg,
                        rem_msg,
                    )
        except Exception:
            pass

print(json.dumps(findings, indent=2))
PYEOF
