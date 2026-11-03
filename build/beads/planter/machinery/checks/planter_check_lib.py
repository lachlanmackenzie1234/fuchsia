#!/usr/bin/env fuchsia-vendored-python
# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

"""Shared helpers for Planter deterministic check scripts and tools."""

import ast
import json
import os
import re
import signal
import subprocess
import time

EXCLUDED_GLOBAL_DIRS = {
    "bundles/assembly",
    "build/bazel",
    "build/bazel2gn",
    "build/images",
    "build/config/rust/lints",
    "third_party/rust_crates",
}

BAZEL_TEST_RULES = {
    "cc_test",
    "fx_cc_test_binary",
    "fx_test",
    "fx_test_component",
    "fx_unittest_component",
    "fx_unittest_package",
    "go_test",
    "host_cc_test",
    "host_go_test",
    "host_py_test",
    "host_rustc_test",
    "host_shac_test",
    "host_test",
    "host_test_data_files",
    "rust_test",
    "rustc_test",
    "wrap_host_rust_test",
}

GN_NON_TARGET_KEYWORDS = {
    "assert",
    "declare_args",
    "else",
    "foreach",
    "forward_variables_from",
    "if",
    "import",
    "pool",
    "set_default_toolchain",
    "set_defaults",
    "template",
    "toolchain",
}

NOT_TARGETS = GN_NON_TARGET_KEYWORDS

SENTINEL_RE = re.compile(
    r"^[ \t]*(?:##\s*BAZEL2GN SENTINEL[^\n]*|#LOCAL_BAZEL_BUILD_SENTINEL[^\n]*)",
    re.MULTILINE,
)

_GN_CALL_RE = re.compile(r'\b([A-Za-z_]\w*)\(\s*"([^"$]+)"\s*\)\s*\{')
_WARNING_SEVERITIES = {"WARNING", "WARN", "MINOR", "MEDIUM", "MODERATE"}
_INFO_SEVERITIES = {
    "INFO",
    "INFORMATIONAL",
    "NOTE",
    "NIT",
    "NITPICK",
    "SUGGESTION",
    "OPTIONAL",
    "LOW",
    "FYI",
}


def normalize_severity(severity):
    s = (severity or "").strip().upper()
    if s in _WARNING_SEVERITIES:
        return "WARNING"
    if s in _INFO_SEVERITIES:
        return "INFO"
    return "ERROR"


def emit(
    findings,
    category,
    severity,
    file="",
    line=0,
    message="",
    remediation="",
    source=None,
):
    """Appends a standardized finding dictionary to `findings` and returns it."""
    src = (
        source
        or os.environ.get("PLANTER_CHECK_NAME", "").strip()
        or "planter_check"
    )
    raw_sev = (severity or "").strip()
    sev_out = (
        raw_sev
        if raw_sev in ("error", "warning", "info", "ERROR", "WARNING", "INFO")
        else normalize_severity(raw_sev)
    )
    entry = {
        "source": src,
        "category": category or "",
        "severity": sev_out,
        "file": file or "",
        "line": int(line or 0),
        "message": message or "",
        "remediation": remediation or "",
    }
    if findings is not None:
        findings.append(entry)
    return entry


def emit_json(findings):
    """Prints `findings` as formatted JSON to stdout."""
    print(json.dumps(findings, indent=2))


def normalize_target_dir(raw_dir, workdir=None):
    """Normalizes a package directory argument to a checkout-relative path."""
    d = (raw_dir or "").strip()
    if d.startswith("//"):
        d = d[2:]
    elif workdir and os.path.isabs(d):
        rel = os.path.relpath(os.path.abspath(d), os.path.abspath(workdir))
        if rel != ".." and not rel.startswith("../"):
            d = rel
    while d.startswith("./"):
        d = d[2:]
    d = d.strip("/")
    return "" if d == "." else d


def _git_out(workdir, args):
    try:
        return subprocess.check_output(
            ["git", "-C", workdir] + list(args),
            stderr=subprocess.DEVNULL,
            text=True,
        ).strip()
    except Exception:
        return ""


def _is_ancestor(workdir, anc, desc):
    if not anc or not desc:
        return False
    try:
        return (
            subprocess.run(
                [
                    "git",
                    "-C",
                    workdir,
                    "merge-base",
                    "--is-ancestor",
                    anc,
                    desc,
                ],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                check=False,
            ).returncode
            == 0
        )
    except Exception:
        return False


def _resolve_active_target_dirs(workdir, target_dirs=None):
    """Normalizes explicit `target_dirs` or `$PLANTER_TARGET_DIRS` / `$PLANTER_TARGET_DIR`."""
    if target_dirs is not None:
        raw_items = (
            re.split(r"[\s,]+", target_dirs)
            if isinstance(target_dirs, str)
            else list(target_dirs)
        )
    else:
        raw_env = (
            os.environ.get("PLANTER_TARGET_DIRS", "").strip()
            or os.environ.get("PLANTER_TARGET_DIR", "").strip()
        )
        raw_items = re.split(r"[\s,]+", raw_env) if raw_env else []
    out = []
    for item in raw_items:
        norm = normalize_target_dir(item, workdir)
        if norm and norm not in out:
            out.append(norm)
    return out


def resolve_change_base(workdir, change_base=None, target_dirs=None):
    """Deterministically resolves the git change base for the current check run."""
    env_resolved = os.environ.get("PLANTER_RESOLVED_CHANGE_BASE", "").strip()
    if env_resolved and change_base is None:
        return env_resolved

    raw = (
        change_base
        if change_base is not None
        else os.environ.get("PLANTER_CHANGE_BASE", "")
    )
    base = (raw or "").strip() or "HEAD"
    if env_resolved and base in (
        "HEAD",
        os.environ.get("PLANTER_CHANGE_BASE", "").strip(),
    ):
        return env_resolved

    m_jj = re.fullmatch(r"@(-*)", base)
    if m_jj:
        base = f"HEAD~{len(m_jj.group(1))}" if m_jj.group(1) else "HEAD"

    if not workdir or not os.path.isdir(workdir):
        return base

    head_sha = _git_out(
        workdir, ["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]
    )
    if not head_sha:
        return base

    parent_sha = _git_out(
        workdir, ["rev-parse", "--verify", "--quiet", "HEAD~1^{commit}"]
    )
    base_sha = _git_out(
        workdir, ["rev-parse", "--verify", "--quiet", f"{base}^{{commit}}"]
    )
    origin_main = _git_out(
        workdir, ["rev-parse", "--verify", "--quiet", "origin/main^{commit}"]
    )
    merge_base = (
        _git_out(workdir, ["merge-base", origin_main, head_sha])
        if (origin_main and head_sha)
        else ""
    )

    if (
        base_sha
        and merge_base
        and base_sha != merge_base
        and _is_ancestor(workdir, base_sha, merge_base)
    ):
        base = merge_base
        base_sha = merge_base
    elif (
        head_sha and base_sha and not _is_ancestor(workdir, base_sha, head_sha)
    ):
        base = merge_base or ("HEAD~1" if parent_sha else base)
        base_sha = _git_out(
            workdir, ["rev-parse", "--verify", "--quiet", f"{base}^{{commit}}"]
        )

    # Do not step back to HEAD~1 when HEAD is already merged on origin/main
    # (e.g. an upstream [bazel_migration] commit at checkout base while edits
    # are uncommitted) or is a stacked prerequisite commit outside target_dirs.
    head_on_origin = bool(
        origin_main
        and head_sha
        and (head_sha == origin_main or merge_base == head_sha)
    )
    if parent_sha and not head_on_origin:
        subj = _git_out(workdir, ["log", "-1", "--format=%s", "HEAD"])
        is_mig_subj = subj.startswith(
            ("[bazel_migration]", "[bazel2gn]", "[bazel]")
        )
        if is_mig_subj and (base == "HEAD" or base_sha == head_sha):
            tdirs = _resolve_active_target_dirs(workdir, target_dirs)
            head_files = [
                line.strip()
                for line in _git_out(
                    workdir, ["diff", "--name-only", "HEAD~1", "HEAD"]
                ).splitlines()
                if line.strip()
            ]
            active_tdirs = [
                td
                for td in tdirs
                if os.path.exists(os.path.join(workdir, td))
                or any(f == td or f.startswith(td + "/") for f in head_files)
            ]
            if active_tdirs and not any(
                f == td or f.startswith(td + "/")
                for td in active_tdirs
                for f in head_files
            ):
                return base
            return "HEAD~1"

    return base


def changed_files(workdir, change_base=None, target_dirs=None):
    """Returns the list of changed and untracked files relative to `change_base`."""
    if "PLANTER_CHANGED_FILES" in os.environ and (
        change_base is None
        or change_base
        in (
            "HEAD",
            os.environ.get("PLANTER_CHANGE_BASE", "").strip(),
            os.environ.get("PLANTER_RESOLVED_CHANGE_BASE", "").strip(),
        )
    ):
        return [
            line.strip()
            for line in os.environ["PLANTER_CHANGED_FILES"].splitlines()
            if line.strip()
        ]

    if not workdir or not os.path.isdir(workdir):
        return []

    eff_base = resolve_change_base(
        workdir, change_base, target_dirs=target_dirs
    )
    seen = set()
    result = []

    for cmd in (
        ["git", "-C", workdir, "diff", "--name-only", eff_base],
        ["git", "-C", workdir, "diff", "--cached", "--name-only"],
        ["git", "-C", workdir, "ls-files", "--others", "--exclude-standard"],
    ):
        try:
            out = subprocess.check_output(
                cmd, stderr=subprocess.DEVNULL, text=True
            )
        except Exception:
            continue
        for line in out.splitlines():
            rel = line.strip()
            if not rel or rel in seen:
                continue
            bname = os.path.basename(rel)
            if bname.startswith("CoderReport") and bname.endswith(".json"):
                continue
            seen.add(rel)
            result.append(rel)

    return result


_review_diff_cache = {}


def read_review_diff_findings(workdir=None):
    """Loads precomputed review_diff findings from PLANTER_REVIEW_DIFF_JSON, or queries planter diff --local when unset."""
    if "PLANTER_REVIEW_DIFF_JSON" in os.environ:
        path = os.environ.get("PLANTER_REVIEW_DIFF_JSON", "").strip()
        if not path or not os.path.isfile(path):
            return []
        try:
            with open(path, "r", encoding="utf-8") as f:
                data = json.load(f)
            return data if isinstance(data, list) else []
        except Exception:
            return []
    if os.environ.get("PLANTER_SKIP_DIFF_COVERAGE") == "1":
        return []
    wdir = (
        workdir or os.environ.get("PLANTER_WORKDIR", "").strip() or os.getcwd()
    )
    if not wdir or not os.path.isdir(wdir):
        return []
    wdir = os.path.abspath(wdir)
    if wdir in _review_diff_cache:
        return list(_review_diff_cache[wdir])
    try:
        import rdeps_lib as rl

        planter_bin = rl.find_planter_bin()
    except Exception:
        planter_bin = None
    if not planter_bin:
        return []
    env = dict(
        os.environ,
        PLANTER_BIN=planter_bin,
        PLANTER_SKIP_DIFF_COVERAGE="1",
    )
    for flag in ("--format=findings-json", "--check-findings"):
        try:
            out = subprocess.run(
                [planter_bin, "diff", "--local", flag, f"--workdir={wdir}"],
                cwd=wdir,
                env=env,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                text=True,
                timeout=15,
            ).stdout.strip()
            findings, parsed_ok = parse_check_findings(out)
            if parsed_ok or out.startswith("["):
                _review_diff_cache[wdir] = findings
                return list(findings)
        except Exception:
            continue
    return []


def strip_comments(text, blank=False):
    """Strips `#` comments outside string literals while preserving line numbers.

    When `blank=True`, replaces comment characters with spaces so character offsets
    remain identical to `text`.
    """
    if not text:
        return ""
    out = []
    i = 0
    n = len(text)
    quote = None
    while i < n:
        c = text[i]
        if quote is not None:
            if c == "\\" and i + 1 < n:
                out.append(text[i : i + 2])
                i += 2
                continue
            if c == quote:
                quote = None
            out.append(c)
            i += 1
        elif c in ('"', "'"):
            quote = c
            out.append(c)
            i += 1
        elif c == "#":
            j = text.find("\n", i)
            if j < 0:
                j = n
            if blank:
                out.append(" " * (j - i))
            i = j
        else:
            out.append(c)
            i += 1
    return "".join(out)


def blank_comments(text):
    """Replaces `#` comments outside strings with spaces, preserving offsets and lines."""
    return strip_comments(text, blank=True)


def has_sentinel(gn_text):
    """Returns True if `gn_text` contains a BAZEL2GN or LOCAL_BAZEL_BUILD sentinel line."""
    return bool(gn_text and SENTINEL_RE.search(gn_text))


def split_sentinel(gn_text):
    """Splits `gn_text` at the canonical BAZEL2GN SENTINEL line into `(above, below)`.

    If no sentinel line is present, returns `(gn_text, "")`.
    """
    if not gn_text:
        return "", ""
    m = SENTINEL_RE.search(gn_text)
    if not m:
        return gn_text, ""
    return gn_text[: m.start()], gn_text[m.end() :]


def find_closing_delim(code, open_idx, open_ch="{", close_ch="}"):
    """Returns the index of `close_ch` matching `open_ch` at `code[open_idx]`, ignoring strings."""
    depth = 0
    quote = None
    i = open_idx
    n = len(code)
    while i < n:
        c = code[i]
        if quote is not None:
            if c == "\\" and i + 1 < n:
                i += 2
                continue
            if c == quote:
                quote = None
        elif c in ('"', "'"):
            quote = c
        elif c == open_ch:
            depth += 1
        elif c == close_ch:
            depth -= 1
            if depth == 0:
                return i
        i += 1
    return n


def extract_gn_targets(gn_text, include_templates=False):
    """Extracts GN target blocks from `gn_text`.

    Returns a list of dicts with keys:
      `type`, `tmpl`, `rule`, `name`, `line`, `body`, `clean_body`, `start`, `end`.
    """
    if not gn_text:
        return []
    code = blank_comments(gn_text)
    targets = []
    for m in _GN_CALL_RE.finditer(code):
        tmpl = m.group(1)
        name = m.group(2)
        if tmpl in GN_NON_TARGET_KEYWORDS and not (
            include_templates and tmpl == "template"
        ):
            continue
        brace_idx = m.end() - 1
        close_idx = find_closing_delim(code, brace_idx, "{", "}")
        line_no = code.count("\n", 0, m.start(1)) + 1
        raw_body = gn_text[brace_idx + 1 : close_idx]
        clean_body = code[brace_idx + 1 : close_idx]
        targets.append(
            {
                "type": tmpl,
                "tmpl": tmpl,
                "rule": tmpl,
                "name": name,
                "line": line_no,
                "body": raw_body,
                "clean_body": clean_body,
                "start": m.start(1),
                "end": min(len(gn_text), close_idx + 1),
            }
        )
    return targets


def has_bazel2gn_skip_comment(src_lines, call_lineno):
    """Returns True if the rule call at 1-based `call_lineno` is preceded by `# @bazel2gn:skip`."""
    idx = call_lineno - 2
    while idx >= 0:
        stripped_line = src_lines[idx].strip()
        if not stripped_line.startswith("#"):
            break
        if re.search(r"^#\s*@bazel2gn:skip\b", stripped_line):
            return True
        idx -= 1
    return False


def _extract_ast_strings(node, var_table=None, depth=0):
    if node is None or depth > 5:
        return []
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        return [node.value]
    if var_table and isinstance(node, ast.Name) and node.id in var_table:
        return _extract_ast_strings(var_table[node.id], var_table, depth + 1)
    if isinstance(node, (ast.List, ast.Tuple, ast.Set)):
        out = []
        for elt in node.elts:
            out.extend(_extract_ast_strings(elt, var_table, depth + 1))
        return out
    if isinstance(node, ast.BinOp) and isinstance(node.op, ast.Add):
        return _extract_ast_strings(
            node.left, var_table, depth + 1
        ) + _extract_ast_strings(node.right, var_table, depth + 1)
    if isinstance(node, ast.Call):
        out = []
        for arg in node.args:
            if isinstance(arg, ast.Dict):
                for v in arg.values:
                    out.extend(_extract_ast_strings(v, var_table, depth + 1))
            else:
                out.extend(_extract_ast_strings(arg, var_table, depth + 1))
        for kw in node.keywords:
            out.extend(_extract_ast_strings(kw.value, var_table, depth + 1))
        return out
    return []


def extract_bazel_targets(bazel_text, filename="<BUILD.bazel>"):
    """Extracts Bazel rule targets from `bazel_text`.

    Returns a list of dicts with keys:
      `rule`, `type`, `name`, `line`, `end_line`, `testonly`, `actual`,
      `visibility`, `deps`, `srcs`, `attrs`, `keywords`, `has_bazel2gn_skip`.
    """
    if not bazel_text:
        return []
    src_lines = bazel_text.splitlines()
    try:
        tree = ast.parse(bazel_text, filename=filename)
    except SyntaxError:
        return []

    var_table = {}
    for stmt in tree.body:
        if isinstance(stmt, ast.Assign):
            for tgt in stmt.targets:
                if isinstance(tgt, ast.Name):
                    var_table[tgt.id] = stmt.value

    targets = []
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call):
            continue
        if isinstance(node.func, ast.Name):
            rule = node.func.id
        elif isinstance(node.func, ast.Attribute):
            rule = node.func.attr
        else:
            continue

        tname = None
        actual = None
        is_testonly = False
        kw_map = {}
        for kw in node.keywords:
            if not kw.arg:
                continue
            kw_map[kw.arg] = kw.value
            if (
                kw.arg == "name"
                and isinstance(kw.value, ast.Constant)
                and isinstance(kw.value.value, str)
            ):
                tname = kw.value.value
            elif (
                kw.arg == "actual"
                and isinstance(kw.value, ast.Constant)
                and isinstance(kw.value.value, str)
            ):
                actual = kw.value.value
            elif kw.arg == "testonly":
                if (
                    isinstance(kw.value, ast.Constant)
                    and kw.value.value is True
                ):
                    is_testonly = True
                elif isinstance(kw.value, ast.Name) and kw.value.id in (
                    "True",
                    "true",
                ):
                    is_testonly = True

        if not tname:
            continue

        vis = _extract_ast_strings(kw_map.get("visibility"), var_table)
        deps = _extract_ast_strings(kw_map.get("deps"), var_table)
        srcs = _extract_ast_strings(kw_map.get("srcs"), var_table)
        lineno = getattr(node, "lineno", 1)
        end_lineno = getattr(node, "end_lineno", lineno)
        targets.append(
            {
                "rule": rule,
                "type": rule,
                "name": tname,
                "line": lineno,
                "end_line": end_lineno,
                "testonly": is_testonly,
                "actual": actual,
                "visibility": vis,
                "deps": deps,
                "srcs": srcs,
                "attrs": set(kw_map.keys()),
                "keywords": kw_map,
                "has_bazel2gn_skip": has_bazel2gn_skip_comment(
                    src_lines, lineno
                ),
            }
        )

    targets.sort(key=lambda t: (t["line"], t["name"]))
    return targets


# Mirrors planter's substrate.Finding JSON fields (waiver: see visibility_audit.sh).
FINDING_FIELDS = (
    "source",
    "category",
    "severity",
    "file",
    "line",
    "message",
    "remediation",
    "waiver",
)


def new_finding(**kw):
    """Creates a finding dict with default empty fields."""
    f = {k: "" for k in FINDING_FIELDS}
    f["line"] = 0
    f.update(kw)
    return f


def is_blocking_finding(finding):
    """Returns True if `finding` has ERROR or WARNING severity."""
    return normalize_severity(finding.get("severity", "")) in (
        "ERROR",
        "WARNING",
    )


def _reject_constant(name):
    raise ValueError(f"invalid JSON constant {name}")


def _go_field(obj, name):
    value = None
    for k, v in obj.items():
        if (k == name or k.lower() == name) and v is not None:
            value = v
    return value


def load_check_manifest(path):
    """Parses `checks/manifest.json` using Go-compatible case-insensitive field matching."""
    try:
        with open(path, encoding="utf-8") as f:
            text = f.read()
    except OSError:
        return None
    try:
        doc = json.loads(text, parse_constant=_reject_constant)
    except ValueError as e:
        raise ValueError(f"invalid JSON: {e}") from e
    if doc is None:
        return []
    if not isinstance(doc, dict):
        raise ValueError("the manifest is not a JSON object")
    version = _go_field(doc, "version")
    if version is not None and (
        isinstance(version, bool) or not isinstance(version, int)
    ):
        raise ValueError('"version" is not an integer')
    checks = _go_field(doc, "checks")
    if checks is None:
        return []
    if not isinstance(checks, list):
        raise ValueError('"checks" is not a list')
    specs = []
    for n, c in enumerate(checks):
        c = {} if c is None else c
        if not isinstance(c, dict):
            raise ValueError(f"checks[{n}] is not an object")
        spec = {
            "name": "",
            "description": "",
            "script_file": "",
            "args": [],
            "enabled": False,
        }
        for field in ("name", "description", "script_file"):
            v = _go_field(c, field)
            if v is not None:
                if not isinstance(v, str):
                    raise ValueError(f"checks[{n}].{field} is not a string")
                spec[field] = v
        v = _go_field(c, "args")
        if v is not None:
            if not isinstance(v, list) or not all(
                a is None or isinstance(a, str) for a in v
            ):
                raise ValueError(f"checks[{n}].args is not a list of strings")
            spec["args"] = ["" if a is None else a for a in v]
        v = _go_field(c, "enabled")
        if v is not None:
            if not isinstance(v, bool):
                raise ValueError(f"checks[{n}].enabled is not a boolean")
            spec["enabled"] = v
        specs.append(spec)
    return specs


def _decode_findings(value):
    ok = True
    out = []
    for el in value:
        f = new_finding()
        if isinstance(el, dict):
            for k, v in el.items():
                name = k if k in f else k.lower() if k.lower() in f else None
                if name is None or v is None:
                    continue
                if name == "line":
                    if (
                        isinstance(v, int)
                        and not isinstance(v, bool)
                        and -(1 << 63) <= v < (1 << 63)
                    ):
                        f["line"] = v
                    else:
                        ok = False
                elif isinstance(v, str):
                    f[name] = v
                else:
                    ok = False
        elif el is not None:
            ok = False
        out.append(f)
    return out, ok


def parse_check_findings(raw):
    """Parses JSON findings output from a check script, recovering trailing JSON arrays if needed."""
    if not raw:
        return [], False
    partial = []
    try:
        whole = json.loads(raw, parse_constant=_reject_constant)
    except ValueError:
        whole = ValueError
    if whole is None:
        return [], True
    if isinstance(whole, list):
        findings, ok = _decode_findings(whole)
        if ok:
            return findings, True
        partial = findings
    decoder = json.JSONDecoder(parse_constant=_reject_constant)
    candidates = []
    start = raw.find("[")
    while start != -1:
        try:
            value, end = decoder.raw_decode(raw, start)
            candidates.append((end, start, value))
        except ValueError:
            pass
        start = raw.find("[", start + 1)
    candidates.sort(key=lambda c: (-c[0], c[1]))
    for _, _, value in candidates:
        findings, ok = _decode_findings(value)
        if ok:
            return findings, True
        partial = findings
    return partial, False


SIGNAL_NAMES = {
    signal.SIGHUP: "hangup",
    signal.SIGINT: "interrupt",
    signal.SIGQUIT: "quit",
    signal.SIGABRT: "aborted",
    signal.SIGBUS: "bus error",
    signal.SIGFPE: "floating point exception",
    signal.SIGKILL: "killed",
    signal.SIGSEGV: "segmentation fault",
    signal.SIGPIPE: "broken pipe",
    signal.SIGTERM: "terminated",
}


def fmt_secs(secs):
    """Formats seconds as e.g. '1.2s' or '2m05s'."""
    return (
        f"{secs:.1f}s"
        if secs < 60
        else f"{int(secs // 60)}m{int(secs % 60):02d}s"
    )


def _stop_proc_tree(proc):
    if proc.poll() is not None:
        return
    children = {}
    for entry in os.listdir("/proc") if os.path.isdir("/proc") else []:
        if entry.isdigit():
            try:
                with open(f"/proc/{entry}/stat") as f:
                    stat = f.read()
                ppid = int(stat[stat.rindex(")") + 2 :].split()[1])
                children.setdefault(ppid, []).append(int(entry))
            except (OSError, ValueError, IndexError):
                pass
    pids, todo = [proc.pid], [proc.pid]
    while todo:
        for child in children.get(todo.pop(), []):
            pids.append(child)
            todo.append(child)
    for sig, wait_s in ((signal.SIGTERM, 10), (signal.SIGKILL, None)):
        for pid in pids:
            try:
                os.kill(pid, sig)
            except OSError:
                pass
        if wait_s:
            try:
                proc.wait(timeout=wait_s)
                return
            except subprocess.TimeoutExpired:
                pass
    proc.wait()


def execute_subprocess(cmd, cwd, env, label="", heartbeat_secs=60, log_fn=None):
    """Runs `cmd` with periodic heartbeat logs and full process-tree cleanup on interrupt."""
    try:
        proc = subprocess.Popen(
            cmd,
            cwd=cwd,
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
    except OSError as e:
        return "", "", str(e)
    started = time.monotonic()
    try:
        while True:
            try:
                out, err = proc.communicate(timeout=heartbeat_secs)
                break
            except subprocess.TimeoutExpired:
                if log_fn:
                    log_fn(
                        f"{label} still running ({fmt_secs(time.monotonic() - started)})"
                    )
    except BaseException:
        _stop_proc_tree(proc)
        raise
    rc = proc.returncode
    err_text = (
        None
        if rc == 0
        else (
            f"exit status {rc}"
            if rc > 0
            else f"signal: {SIGNAL_NAMES.get(-rc, str(-rc))}"
        )
    )
    return (
        out.decode("utf-8", "replace"),
        err.decode("utf-8", "replace"),
        err_text,
    )
