#!/usr/bin/env bash
# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

set -uo pipefail

# Runs the deterministic checks of checks/manifest.json the way planter runs them
# after the coder reports: same scripts, arguments, environment, order, JSON
# parsing, severity normalization and de-duplication.
#
# Usage: run_checks.sh [--skip-build] [--only NAME[,NAME...]] [--json] [--list] [DIR...]

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="${PLANTER_PROJECT_DIR:-$(dirname "$SCRIPT_DIR")}"

python3 - "$PROJECT_DIR" "$@" <<'PYEOF'
import ast
import atexit
from concurrent.futures import ThreadPoolExecutor
import json
import os
import re
import signal
import subprocess
import sys
import tempfile
import time

USAGE = """\
usage: run_checks.sh [--skip-build] [--only NAME[,NAME...]] [--json] [--list] [DIR...]

Runs the checks in checks/manifest.json the way planter runs them after the
coder reports. Exits 1 if any ERROR or WARNING finding would send the change
back, 0 otherwise.

  --skip-build  set PLANTER_SKIP_BUILD=1 so build_verification does not build
  --only NAMES  run only these checks (comma-separated, repeatable)
  --json        print all findings as one JSON array instead of a report
  --list        list the checks and exit
  DIR...        target directories (default: $PLANTER_TARGET_DIRS)"""

SERIAL_BUILD_CHECKS = ("build_verification", "tests_json_parity")
# Must match planter's upload gate (mergeReviewDiffFindings): only visibility_narrowed
# honors check waivers. Listing other review_diff categories here hides blocking
# diffengine findings during coder runs that planter's upload gate still rejects.
WAIVABLE_REVIEW_DIFF_CATEGORIES = {"visibility_narrowed"}

sys.stdout.reconfigure(errors="replace")
sys.stderr.reconfigure(errors="replace")


def log(msg):
    print(f"[run_checks] {msg}", file=sys.stderr, flush=True)


def die(msg, show_usage=False):
    suffix = f"\n\n{USAGE}" if show_usage else ""
    print(f"run_checks: {msg}{suffix}", file=sys.stderr)
    sys.exit(2)


project_dir = os.path.abspath(sys.argv[1])
alt_root = os.path.join(project_dir, "machinery")
mroot = (
    alt_root
    if os.path.isdir(alt_root)
    and not os.path.islink(alt_root)
    and os.path.exists(os.path.join(alt_root, "prompts", "coder.md"))
    and not os.path.exists(os.path.join(project_dir, "prompts", "coder.md"))
    else project_dir
)

sys.path.insert(0, os.path.join(mroot, "checks"))
import planter_check_lib as pcl
import rdeps_lib as rl

skip_build = as_json = list_only = False
only, dir_args, args, i = [], [], sys.argv[2:], 0
while i < len(args):
    a = args[i]
    if a in ("-h", "--help"):
        print(USAGE)
        sys.exit(0)
    elif a == "--skip-build":
        skip_build = True
    elif a == "--json":
        as_json = True
    elif a == "--list":
        list_only = True
    elif a == "--only" or a.startswith("--only="):
        if a == "--only":
            i += 1
            if i >= len(args):
                die("--only needs a comma-separated list of check names", True)
            value = args[i]
        else:
            value = a[len("--only="):]
        names = [n.strip() for n in value.split(",") if n.strip()]
        if not names:
            die("--only needs a comma-separated list of check names", True)
        only.extend(names)
    elif a == "--":
        dir_args.extend(args[i + 1:])
        break
    elif a.startswith("-"):
        die(f"unknown option {a}", True)
    else:
        dir_args.append(a)
    i += 1

manifest_path = os.path.join(mroot, "checks", "manifest.json")
try:
    specs = pcl.load_check_manifest(manifest_path)
except ValueError as e:
    die(f"cannot parse {manifest_path} (planter would fail the same way): {e}")
if specs is None:
    log(f"no readable {manifest_path}; planter runs no checks")
    specs = []

if list_only:
    for s in specs:
        print(f"{s['name']}{'' if s['enabled'] else ' [disabled]'}: {s['description']}")
    sys.exit(0)

enabled = [s for s in specs if s["enabled"]]
disabled = [s for s in specs if not s["enabled"]]
if only:
    known = {s["name"] for s in specs} | {"review_diff"}
    unknown = [n for n in only if n not in known]
    if unknown:
        die(f"unknown check(s): {', '.join(unknown)}; checks: {', '.join(s['name'] for s in specs)}, review_diff", True)
    wanted = set(only)
    enabled = [s for s in enabled if s["name"] in wanted]
    disabled = [s for s in disabled if s["name"] in wanted]

workdir = os.environ.get("PLANTER_WORKDIR", "") or os.getcwd()
if dir_args:
    target_dirs = [d for d in (pcl.normalize_target_dir(a, workdir) for a in dir_args) if d]
else:
    target_dirs = [
        pcl.normalize_target_dir(d, workdir)
        for d in os.environ.get("PLANTER_TARGET_DIRS", "").split()
        if pcl.normalize_target_dir(d, workdir)
    ]
    if not target_dirs and os.environ.get("PLANTER_TARGET_DIR", "").strip():
        td = pcl.normalize_target_dir(os.environ["PLANTER_TARGET_DIR"], workdir)
        target_dirs = [td] if td else []

# Fetch origin/main at most once up front, then export PLANTER_SKIP_FETCH=1 to all checks.
if os.environ.get("PLANTER_SKIP_FETCH", "").strip() in ("", "0"):
    for repo in rl.discover_git_repos(workdir):
        rl.maybe_refresh_repo_origin_main(repo)
os.environ["PLANTER_SKIP_FETCH"] = "1"

# Resolve PLANTER_RESOLVED_CHANGE_BASE and PLANTER_CHANGED_FILES once up front.
change_base = pcl.resolve_change_base(workdir)
os.environ["PLANTER_RESOLVED_CHANGE_BASE"] = change_base
if "PLANTER_CHANGED_FILES" not in os.environ:
    os.environ["PLANTER_CHANGED_FILES"] = "\n".join(pcl.changed_files(workdir, change_base))

signal.signal(signal.SIGTERM, lambda signum, _frame: sys.exit(128 + signum))


def execute(cmd, cwd, env, label):
    return pcl.execute_subprocess(cmd, cwd, env, label=label, log_fn=log)


_precomputed_diff_findings = None
macro_dispute_items = []
verified_diff_fp_ids = set()

# Detect Starlark macro and GN label-normalization false positives in review_diff findings.
# Known GN -> Bazel Starlark macro attribute renames that bazel2gn maps back to GN.
MACRO_ATTR_EQUIVALENTS = {
    "api": ("api_file_path",),
    "args": ("test_args",),
    "args_source_root": ("args_crate_root",),
    "args_sources": ("args_srcs",),
    "features": ("crate_features",),
    "inputs": ("compile_data",),
    "public": ("hdrs",),
    "rustenv": ("rustc_env",),
    "rustflags": ("rustc_flags",),
    "sdk_area": ("api_area",),
    "sdk_category": ("sdk_category", "category"),
    "sdk_name": ("idk_name",),
    "source_root": ("crate_root",),
    "sources": ("srcs",),
}

# Host-only Starlark macros/rules that apply host constraints internally and do not
# take explicit `target_compatible_with = HOST_OS_CONSTRAINTS` in BUILD.bazel.
IMPLICIT_HOST_ONLY_MACROS = {
    "cc_binary_host_tool",
    "ffx_plugin",
    "ffx_tool",
    "go_binary_host_tool",
    "host_go_test",
    "host_py_test",
    "host_rustc_test",
    "host_shac_test",
    "host_test",
    "idk_cc_binary_host_tool",
    "idk_ffx_tool",
    "idk_go_binary_host_tool",
    "idk_rustc_binary_host_tool",
    "py_binary_host_tool",
    "rustc_binary_host_tool",
    "wrap_host_rust_test",
}

ATTR_ALIGN_MSG_RE = re.compile(
    r'^Target "([^"]+)" \(Target Attributes\): ([A-Za-z_]\w*) in GN not carried over to Bazel$'
)
HOST_COMPAT_MSG_RE = re.compile(
    r'^Target "([^"]+)" .*missing `target_compatible_with = HOST_(?:OS_)?CONSTRAINTS` in BUILD\.bazel'
)
DEP_ALIGN_UNMATCHED_RE = re.compile(
    r'^Target "([^"]+)" dependencies differ between GN and Bazel:.*?'
    r"unmatched in GN: \[([^\]]+)\]; unmatched in Bazel: \[([^\]]+)\]\s*$"
)
DEP_ALIGN_BAZEL_ONLY_RE = re.compile(
    r'^Target "([^"]+)" dependencies differ between GN and Bazel:'
    r"(?!.*unmatched in GN:).*?unmatched in Bazel: \[([^\]]+)\]\s*$"
)
UNIT_TEST_PARITY_MSG_RE = re.compile(
    r'^Unit tests of "([^"]+)": GN test_deps \[([^\]]+)\] '
    r'are not dependencies of "([^"]+)"\.?\s*$'
)

KNOWN_GN_TARGET_ATTRS = {
    "all_dependent_configs",
    "api",
    "args",
    "args_source_root",
    "args_sources",
    "asmflags",
    "assert_no_deps",
    "category",
    "cflags",
    "cflags_c",
    "cflags_cc",
    "cflags_objc",
    "cflags_objcc",
    "check_cfgs",
    "cm_label",
    "complete_static_lib",
    "component_name",
    "configs",
    "crate_name",
    "crate_root",
    "data",
    "data_deps",
    "defines",
    "depfile",
    "deps",
    "disable_clippy",
    "disable_rbe",
    "disable_syslog_backend",
    "edition",
    "exclude_toolchain_tags",
    "features",
    "include_dirs",
    "inputs",
    "ldflags",
    "lib_dirs",
    "library_name",
    "libs",
    "link_deps",
    "lint_config",
    "main_source",
    "manifest",
    "mnemonic",
    "name",
    "non_rust_deps",
    "non_test_deps",
    "output_dir",
    "output_extension",
    "output_name",
    "outputs",
    "package_name",
    "pool",
    "proc_macro_deps",
    "public",
    "public_configs",
    "public_deps",
    "resources",
    "rustenv",
    "rustflags",
    "script",
    "sdk_area",
    "sdk_category",
    "sdk_headers_for_internal_use",
    "sdk_name",
    "sdk_publishable",
    "source_root",
    "sources",
    "test_deps",
    "testonly",
    "version",
    "visibility",
    "with_unit_tests",
}


# Normalize GN/Bazel dependency labels (collapsing duplicate `/` inside package paths).
def _canon_dep_label(raw, pkg_dir=""):
    lbl = raw.strip().strip('"').strip("'")
    if not lbl:
        return None
    lbl = re.sub(r"\([^()]*\)$", "", lbl).strip()
    if not lbl or "$" in lbl:
        return None
    if lbl.startswith("@//"):
        lbl = lbl[1:]
    if lbl.startswith("@"):
        return None
    if lbl.startswith("//"):
        body = re.sub(r"/+", "/", lbl[2:])
        path, sep, name = body.partition(":")
    elif lbl.startswith(":"):
        if not pkg_dir:
            return None
        path, sep, name = pkg_dir, ":", lbl[1:]
    elif pkg_dir:
        rel, sep, name = re.sub(r"/+", "/", lbl).partition(":")
        path = os.path.normpath(os.path.join(pkg_dir, rel)).replace("\\", "/")
        if path.startswith(".."):
            return None
    else:
        return None
    path = path.strip("/")
    if not path:
        return None
    name = name if sep else path.rsplit("/", 1)[-1]
    return f"//{path}:{name}" if name else None


# Read a package's BUILD.gn at base_rev (before migration edits or deletion).
def _read_base_pkg_gn(wdir, pkg_rel, base_rev):
    if not wdir or not pkg_rel or not base_rev:
        return ""
    try:
        return subprocess.check_output(
            ["git", "-C", wdir, "show", f"{base_rev}:{pkg_rel}/BUILD.gn"],
            stderr=subprocess.DEVNULL,
            text=True,
        )
    except Exception:
        return ""


# Check whether gn_attr was a local GN helper variable in base BUILD.gn whose
# interpolated target attribute(s) are already carried on the Bazel target.
def _is_base_gn_local_helper_var(wdir, pkg_rel, base_rev, gn_attr, bz_attrs):
    if gn_attr in KNOWN_GN_TARGET_ATTRS or not bz_attrs:
        return False
    base_gn = _read_base_pkg_gn(wdir, pkg_rel, base_rev)
    if not base_gn:
        return False
    clean_base = pcl.blank_comments(base_gn)
    if not re.search(rf"(?m)^\s*{re.escape(gn_attr)}\s*=", clean_base):
        return False
    var_ref_re = re.compile(
        rf"(?:\$\{{?{re.escape(gn_attr)}\}}?|(?<![\w.$]){re.escape(gn_attr)}(?![\w]))"
    )
    consumer_attrs = set()
    for am in re.finditer(
        r"(?m)^\s*([A-Za-z_]\w*)\s*\+?=\s*(\[[^\]]*\]|[^\n]+)", clean_base
    ):
        other_attr, rhs = am.group(1), am.group(2)
        if other_attr != gn_attr and var_ref_re.search(rhs):
            consumer_attrs.add(other_attr)
    if not consumer_attrs:
        return False
    return all(
        any(eq in bz_attrs for eq in MACRO_ATTR_EQUIVALENTS.get(ca, (ca,)))
        for ca in consumer_attrs
    )


# Check whether attribute_alignment is a Starlark macro rename or local GN helper var false positive.
def is_macro_synced_attr_false_positive(finding, wdir, base_rev=""):
    """Returns (target_name, gn_attr, kind) when BUILD.bazel / bazel2gn already carries gn_attr."""
    if finding.get("category") != "attribute_alignment":
        return None
    if finding.get("severity") not in ("ERROR", "WARNING"):
        return None
    rel_bz = finding.get("file", "")
    if not rel_bz.endswith("BUILD.bazel") or not wdir:
        return None
    m = ATTR_ALIGN_MSG_RE.match(finding.get("message", ""))
    if not m:
        return None
    tname, gn_attr = m.group(1), m.group(2)
    pkg_rel = os.path.dirname(rel_bz).strip("/")
    pkg_dir = os.path.dirname(os.path.join(wdir, rel_bz))
    var_ref_re = re.compile(
        rf"(?:\$\{{?{re.escape(gn_attr)}\}}?|(?<![\w.$]){re.escape(gn_attr)}(?![\w]))"
    )

    # Check 1: Dual-build BUILD.gn has gn_attr synced or referenced via pre-sentinel var on tname.
    gn_path = os.path.join(pkg_dir, "BUILD.gn")
    if os.path.isfile(gn_path):
        try:
            with open(gn_path, encoding="utf-8") as gf:
                gn_text = gf.read()
        except OSError:
            gn_text = ""
        if pcl.has_sentinel(gn_text):
            above, below = pcl.split_sentinel(gn_text)
            attr_re = re.compile(rf"(?<![\w.]){re.escape(gn_attr)}\s*\+?=")
            above_has_var = (
                gn_attr not in KNOWN_GN_TARGET_ATTRS
                and bool(
                    re.search(
                        rf"(?m)^\s*{re.escape(gn_attr)}\s*=", pcl.blank_comments(above)
                    )
                )
            )
            for gt in pcl.extract_gn_targets(below):
                if gt["name"] != tname:
                    continue
                if attr_re.search(gt["clean_body"]):
                    return (tname, gn_attr, "macro_attr")
                if above_has_var and var_ref_re.search(gt["clean_body"]):
                    return (tname, gn_attr, "local_gn_var")

    # Check 2: BUILD.bazel sets gn_attr counterpart, @bazel2gn var overwrite, or local helper attr.
    bz_path = os.path.join(wdir, rel_bz)
    if os.path.isfile(bz_path):
        try:
            with open(bz_path, encoding="utf-8") as bf:
                bz_text = bf.read()
        except OSError:
            bz_text = ""
        lines = bz_text.splitlines()
        ow_var_re = re.compile(r"#\s*@bazel2gn:(?:value_overwrite|raw_overwrite):(.+)$")
        equivs = MACRO_ATTR_EQUIVALENTS.get(gn_attr, (gn_attr,))
        bz_targets = pcl.extract_bazel_targets(bz_text, filename=rel_bz)
        by_name = {bt["name"]: bt for bt in bz_targets}
        for bt in bz_targets:
            if bt["name"] != tname:
                continue
            if any(a in bt["attrs"] for a in equivs):
                return (tname, gn_attr, "macro_attr")
            # fx_component / fx_test_component delegates component_name and manifest to
            # its referenced compiled_manifest (fx_component_manifest) target.
            cm_node = bt.get("keywords", {}).get("compiled_manifest")
            if isinstance(cm_node, ast.Constant) and isinstance(cm_node.value, str):
                cm_lbl = _canon_dep_label(cm_node.value, pkg_rel)
                cm_name = (
                    cm_lbl.split(":", 1)[1]
                    if cm_lbl and cm_lbl.startswith(f"//{pkg_rel}:")
                    else cm_node.value.lstrip(":")
                )
                cm_bt = by_name.get(cm_name)
                if cm_bt and any(a in cm_bt["attrs"] for a in equivs):
                    return (tname, gn_attr, "macro_attr")
            if gn_attr not in KNOWN_GN_TARGET_ATTRS:
                start_ln = max(1, bt.get("line", 1))
                end_ln = min(len(lines), bt.get("end_line", start_ln))
                for ln in lines[start_ln - 1 : end_ln]:
                    om = ow_var_re.search(ln)
                    if om and var_ref_re.search(om.group(1)):
                        return (tname, gn_attr, "local_gn_var")
                if _is_base_gn_local_helper_var(
                    wdir, pkg_rel, base_rev, gn_attr, bt["attrs"]
                ):
                    return (tname, gn_attr, "local_gn_var")
    return None


# Check whether host_compatibility is a false positive on an implicitly host-only macro.
def is_host_only_macro_compat_false_positive(finding, wdir):
    """Returns (target_name, rule_name) when target uses an implicitly host-only macro."""
    if finding.get("category") != "host_compatibility":
        return None
    if finding.get("severity") not in ("ERROR", "WARNING"):
        return None
    rel_bz = finding.get("file", "")
    if not rel_bz.endswith("BUILD.bazel") or not wdir:
        return None
    m = HOST_COMPAT_MSG_RE.match(finding.get("message", ""))
    if not m:
        return None
    tname = m.group(1)
    bz_path = os.path.join(wdir, rel_bz)
    if not os.path.isfile(bz_path):
        return None
    try:
        with open(bz_path, encoding="utf-8") as bf:
            bz_text = bf.read()
    except OSError:
        return None
    for bt in pcl.extract_bazel_targets(bz_text, filename=rel_bz):
        if bt["name"] == tname and bt["rule"] in IMPLICIT_HOST_ONLY_MACROS:
            return (tname, bt["rule"])
    return None


PURE_GN_GROUP_ATTRS = {"public_deps", "deps", "visibility", "testonly"}
BAZEL_CODE_DEP_ATTRS = (
    "deps",
    "args_deps",
    "plugin_deps",
    "proc_macro_deps",
    "link_deps",
    "implementation_deps",
    "public_deps",
    "embed",
)
BAZEL_DATA_DEP_ATTRS = (
    "data",
    "test_data",
    "compile_data",
)
BAZEL_DEP_LIST_ATTRS = BAZEL_CODE_DEP_ATTRS + BAZEL_DATA_DEP_ATTRS
RUST_CRATE_DEP_RULES = {
    "rustc_library",
    "rust_library",
    "rustc_proc_macro",
    "rust_proc_macro",
}


# Normalize vendored Rust crate Bazel labels to GN crate labels for parity comparison.
def _norm_vendor_crate_label(canon_lbl):
    if canon_lbl and canon_lbl.startswith("//third_party/rust_crates/vendor:"):
        crate = canon_lbl.split(":", 1)[1]
        return f"//third_party/rust_crates:{crate}"
    return canon_lbl


# Read a dependency package's BUILD.gn from disk, falling back to base_rev when deleted.
def _read_dep_pkg_gn(wdir, dep_pkg, base_rev):
    gn_path = os.path.join(wdir, dep_pkg, "BUILD.gn")
    if os.path.isfile(gn_path):
        try:
            with open(gn_path, encoding="utf-8") as gf:
                return gf.read()
        except OSError:
            return ""
    if not base_rev:
        return ""
    try:
        return subprocess.check_output(
            ["git", "-C", wdir, "show", f"{base_rev}:{dep_pkg}/BUILD.gn"],
            stderr=subprocess.DEVNULL,
            text=True,
        )
    except Exception:
        return ""


# Expand a pure GN forwarding `group("<name>")` (no configs or link settings) to its targets.
def _expand_pure_gn_forwarding_group(canon_lbl, wdir, base_rev, stop_set, seen=()):
    norm_lbl = _norm_vendor_crate_label(canon_lbl)
    if norm_lbl in stop_set:
        return {norm_lbl}
    if not canon_lbl or not wdir or canon_lbl in seen or len(seen) >= 6:
        return None
    dep_pkg, _, dep_name = canon_lbl[2:].partition(":")
    if not dep_pkg or not dep_name:
        return None
    gn_text = _read_dep_pkg_gn(wdir, dep_pkg, base_rev)
    if not gn_text:
        return None
    blocks = [gt for gt in pcl.extract_gn_targets(gn_text) if gt["name"] == dep_name]
    if len(blocks) != 1 or blocks[0]["tmpl"] != "group":
        return None
    clean_body = blocks[0]["clean_body"]
    attrs = set(re.findall(r"(?m)^\s*([A-Za-z_]\w*)\s*[+-]?=", clean_body))
    if not attrs or not (attrs <= PURE_GN_GROUP_ATTRS):
        return None
    if re.search(r"(?m)^\s*[A-Za-z_]\w*\s*-=", clean_body) or re.search(
        r"\bif\s*\(", clean_body
    ):
        return None
    raw_deps = []
    for lm in re.finditer(r"(?<![\w.])(?:public_deps|deps)\s*\+?=\s*\[", clean_body):
        open_idx = lm.end() - 1
        close_idx = pcl.find_closing_delim(clean_body, open_idx, "[", "]")
        inner = clean_body[open_idx + 1 : close_idx]
        stripped_inner = re.sub(r'"(?:[^"\\]|\\.)*"', "", inner)
        if re.sub(r"[\s,]+", "", stripped_inner):
            return None
        raw_deps.extend(re.findall(r'"((?:[^"\\]|\\.)*)"', inner))
    if not raw_deps:
        return None
    expanded = set()
    next_seen = seen + (canon_lbl,)
    for raw in raw_deps:
        sub_canon = _canon_dep_label(raw, dep_pkg)
        if not sub_canon:
            return None
        sub_norm = _norm_vendor_crate_label(sub_canon)
        if sub_norm in stop_set:
            expanded.add(sub_norm)
            continue
        sub_exp = _expand_pure_gn_forwarding_group(
            sub_canon, wdir, base_rev, stop_set, next_seen
        )
        if sub_exp:
            expanded.update(sub_exp)
        else:
            expanded.add(sub_norm)
    return expanded if expanded else None


# Collect canonical Bazel deps and @bazel2gn:path_overwrite mappings on target `tname`.
def _collect_bazel_target_dep_context(wdir, rel_bz, tname, pkg_rel):
    bz_path = os.path.join(wdir, rel_bz)
    if not os.path.isfile(bz_path):
        return set(), {}
    try:
        with open(bz_path, encoding="utf-8") as bf:
            bz_text = bf.read()
    except OSError:
        return set(), {}
    existing = set()
    overwrites = {}
    lines = bz_text.splitlines()
    ow_re = re.compile(r'"([^"]+)"\s*,?\s*#\s*@bazel2gn:path_overwrite:(\S+)')
    for bt in pcl.extract_bazel_targets(bz_text, filename=rel_bz):
        if bt["name"] != tname:
            continue
        for d in bt.get("deps", ()):
            c = _canon_dep_label(d, pkg_rel)
            if c:
                existing.add(_norm_vendor_crate_label(c))
        start_ln = max(1, bt.get("line", 1))
        end_ln = min(len(lines), bt.get("end_line", start_ln))
        block_text = "\n".join(lines[start_ln - 1 : end_ln])
        for attr in BAZEL_DEP_LIST_ATTRS:
            for am in re.finditer(
                rf"(?m)^\s*{re.escape(attr)}\s*=\s*\[([^\]]*)\]", block_text
            ):
                for raw in re.findall(r'"([^"]+)"', am.group(1)):
                    c = _canon_dep_label(raw, pkg_rel)
                    if c:
                        existing.add(_norm_vendor_crate_label(c))
        for ln in lines[start_ln - 1 : end_ln]:
            om = ow_re.search(ln)
            if om:
                bz_c = _canon_dep_label(om.group(1), pkg_rel)
                gn_c = _canon_dep_label(om.group(2), pkg_rel)
                if bz_c and gn_c:
                    overwrites.setdefault(_norm_vendor_crate_label(gn_c), set()).add(
                        _norm_vendor_crate_label(bz_c)
                    )
    return existing, overwrites


# Verify whether Bazel-only unmatched deps come from a shared `select(...)` list
# that was already present in conditional GN `deps` at base_rev and synced via bazel2gn.
def _is_conditional_select_dep_false_positive(wdir, pkg_rel, base_rev, bz_canon):
    if not wdir or not pkg_rel or not bz_canon:
        return False
    base_gn = _read_base_pkg_gn(wdir, pkg_rel, base_rev)
    if not base_gn:
        return False
    base_norms = {
        _norm_vendor_crate_label(c)
        for raw in re.findall(r'"((?:[^"\\]|\\.)*)"', pcl.blank_comments(base_gn))
        for c in (_canon_dep_label(raw, pkg_rel),)
        if c
    }
    bz_norms = {_norm_vendor_crate_label(c) for c in bz_canon}
    if not (bz_norms <= base_norms):
        return False
    gn_path = os.path.join(wdir, pkg_rel, "BUILD.gn")
    if os.path.isfile(gn_path):
        try:
            with open(gn_path, encoding="utf-8") as gf:
                cur_gn = gf.read()
        except OSError:
            cur_gn = ""
        if pcl.has_sentinel(cur_gn):
            _, below = pcl.split_sentinel(cur_gn)
            below_norms = {
                _norm_vendor_crate_label(c)
                for raw in re.findall(r'"((?:[^"\\]|\\.)*)"', pcl.blank_comments(below))
                for c in (_canon_dep_label(raw, pkg_rel),)
                if c
            }
            return bz_norms <= below_norms
    return True


# Check whether dep_alignment WARNING is a verified label-normalization, pure GN
# forwarding-group expansion, conditional select(), or @bazel2gn:path_overwrite false positive.
def is_verified_dep_alignment_false_positive(finding, wdir="", base_rev=""):
    """Returns (target_name, kind) when unmatched GN/Bazel dep lists are equivalent."""
    if finding.get("category") != "dep_alignment":
        return None
    if finding.get("severity") != "WARNING":
        return None
    rel_bz = finding.get("file", "")
    if not rel_bz.endswith("BUILD.bazel"):
        return None
    pkg_rel = os.path.dirname(rel_bz).strip("/")
    m_bz_only = DEP_ALIGN_BAZEL_ONLY_RE.match(finding.get("message", ""))
    if m_bz_only:
        tname = m_bz_only.group(1)
        bz_items = [s.strip() for s in m_bz_only.group(2).split(",") if s.strip()]
        bz_canon = [_canon_dep_label(s, pkg_rel) for s in bz_items]
        if (
            bz_canon
            and None not in bz_canon
            and _is_conditional_select_dep_false_positive(
                wdir, pkg_rel, base_rev, bz_canon
            )
        ):
            return (tname, "conditional_select")
        return None
    m = DEP_ALIGN_UNMATCHED_RE.match(finding.get("message", ""))
    if not m:
        return None
    tname = m.group(1)
    gn_items = [s.strip() for s in m.group(2).split(",") if s.strip()]
    bz_items = [s.strip() for s in m.group(3).split(",") if s.strip()]
    gn_canon = [_canon_dep_label(s, pkg_rel) for s in gn_items]
    bz_canon = [_canon_dep_label(s, pkg_rel) for s in bz_items]
    if not gn_canon or not bz_canon or None in gn_canon or None in bz_canon:
        return None
    if set(gn_canon) == set(bz_canon):
        return (tname, "slash_or_shorthand")
    if not wdir:
        return None
    existing_bz, overwrites = _collect_bazel_target_dep_context(
        wdir, rel_bz, tname, pkg_rel
    )
    bz_norm_set = {_norm_vendor_crate_label(c) for c in bz_canon}
    stop_set = bz_norm_set | existing_bz
    expanded_gn = set()
    for g_lbl in gn_canon:
        g_norm = _norm_vendor_crate_label(g_lbl)
        if g_norm in stop_set:
            expanded_gn.add(g_norm)
        elif g_norm in overwrites and (overwrites[g_norm] & stop_set):
            expanded_gn.update(overwrites[g_norm] & stop_set)
        else:
            exp = _expand_pure_gn_forwarding_group(g_lbl, wdir, base_rev, stop_set)
            if not exp:
                return None
            expanded_gn.update(exp)
    if expanded_gn <= stop_set and bz_norm_set <= expanded_gn:
        return (tname, "forwarding_group")
    return None


# Extract top-level Starlark variable assignments for AST string resolution.
def _parse_bazel_var_table(bz_text, filename="<BUILD.bazel>"):
    try:
        tree = ast.parse(bz_text, filename=filename)
    except SyntaxError:
        return {}
    var_table = {}
    for stmt in tree.body:
        if isinstance(stmt, ast.Assign):
            for tgt in stmt.targets:
                if isinstance(tgt, ast.Name):
                    var_table[tgt.id] = stmt.value
    return var_table


# Check whether canon_lbl resolves to a Rust crate/proc-macro target requiring `deps`.
def _is_bazel_rust_crate_target(wdir, canon_lbl):
    if not canon_lbl or not canon_lbl.startswith("//"):
        return False
    if canon_lbl.startswith(
        (
            "//third_party/rust_crates:",
            "//third_party/rust_crates/vendor:",
            "//third_party/rust_crates/forks/",
        )
    ):
        return True
    dep_pkg, _, dep_name = canon_lbl[2:].partition(":")
    if not dep_pkg or not dep_name:
        return False
    if dep_name.endswith(
        ("_rust", "_rust_flex", "_rust_fdomain", "_rust_fdomain_flex", "_rust_next")
    ):
        return True
    bz_path = os.path.join(wdir, dep_pkg, "BUILD.bazel")
    if not os.path.isfile(bz_path):
        return False
    try:
        with open(bz_path, encoding="utf-8") as bf:
            bz_text = bf.read()
    except OSError:
        return False
    for bt in pcl.extract_bazel_targets(bz_text, filename=bz_path):
        if bt["name"] == dep_name and (
            bt["rule"] in RUST_CRATE_DEP_RULES or bt["rule"] == "fidl_library"
        ):
            return True
    return False


# Collect code_deps, data_deps, and @bazel2gn:path_overwrite mappings for unit test target.
def _collect_unit_test_bazel_context(wdir, rel_bz, lib_name, test_name, pkg_rel):
    bz_path = os.path.join(wdir, rel_bz)
    if not os.path.isfile(bz_path):
        return None
    try:
        with open(bz_path, encoding="utf-8") as bf:
            bz_text = bf.read()
    except OSError:
        return None
    var_table = _parse_bazel_var_table(bz_text, filename=rel_bz)
    bz_targets = pcl.extract_bazel_targets(bz_text, filename=rel_bz)
    by_name = {bt["name"]: bt for bt in bz_targets}
    matched_names = set()
    if test_name in by_name:
        matched_names.add(test_name)
        bin_vals = pcl._extract_ast_strings(
            by_name[test_name]["keywords"].get("binary_name"), var_table
        )
        for bv in bin_vals:
            bname = bv.lstrip(":")
            if bname in by_name:
                matched_names.add(bname)
    for bt in bz_targets:
        if bt["rule"] == "wrap_host_rust_test":
            bin_vals = pcl._extract_ast_strings(
                bt["keywords"].get("binary_name"), var_table
            )
            if any(bv.lstrip(":") == test_name for bv in bin_vals):
                matched_names.add(bt["name"])
    if not matched_names:
        return None

    code_deps = set()
    data_deps = set()
    # Runtime test data placed in `data` of a host_rustc_test (forwarded to the
    # inner rustc_test binary) instead of `test_data` (forwarded to the
    # host_test() wrapper). These are real findings, not false positives.
    host_test_misplaced_data = set()
    overwrites = {}
    lines = bz_text.splitlines()
    ow_re = re.compile(r'"([^"]+)"\s*,?\s*#\s*@bazel2gn:path_overwrite:(\S+)')
    for t_nm in matched_names:
        bt = by_name[t_nm]
        kws = bt.get("keywords", {})
        for attr in BAZEL_CODE_DEP_ATTRS:
            for raw in pcl._extract_ast_strings(kws.get(attr), var_table):
                c = _canon_dep_label(raw, pkg_rel)
                if c:
                    code_deps.add(_norm_vendor_crate_label(c))
        for attr in BAZEL_DATA_DEP_ATTRS:
            for raw in pcl._extract_ast_strings(kws.get(attr), var_table):
                c = _canon_dep_label(raw, pkg_rel)
                if not c:
                    continue
                if attr == "data" and bt["rule"] == "host_rustc_test":
                    host_test_misplaced_data.add(_norm_vendor_crate_label(c))
                else:
                    data_deps.add(_norm_vendor_crate_label(c))
        start_ln = max(1, bt.get("line", 1))
        end_ln = min(len(lines), bt.get("end_line", start_ln))
        for ln in lines[start_ln - 1 : end_ln]:
            om = ow_re.search(ln)
            if om:
                bz_c = _canon_dep_label(om.group(1), pkg_rel)
                gn_c = _canon_dep_label(om.group(2), pkg_rel)
                if bz_c and gn_c:
                    overwrites.setdefault(_norm_vendor_crate_label(gn_c), set()).add(
                        _norm_vendor_crate_label(bz_c)
                    )
    if lib_name in by_name:
        lib_kws = by_name[lib_name].get("keywords", {})
        for attr in ("test_data", "test_compile_data"):
            for raw in pcl._extract_ast_strings(lib_kws.get(attr), var_table):
                c = _canon_dep_label(raw, pkg_rel)
                if c:
                    data_deps.add(_norm_vendor_crate_label(c))

    # Expand same-package host_test_data_files / host_test_data_map targets in data_deps.
    local_prefix = f"//{pkg_rel}:" if pkg_rel else "//:"
    for d_lbl in list(data_deps):
        if not d_lbl.startswith(local_prefix):
            continue
        htd = by_name.get(d_lbl[len(local_prefix) :])
        if not htd or htd["rule"] not in ("host_test_data_files", "host_test_data_map"):
            continue
        hkws = htd.get("keywords", {})
        for raw in pcl._extract_ast_strings(hkws.get("srcs"), var_table):
            c = _canon_dep_label(raw, pkg_rel)
            if c:
                data_deps.add(_norm_vendor_crate_label(c))
        fmap = hkws.get("files_map")
        if isinstance(fmap, ast.Dict):
            for vnode in fmap.values:
                for raw in pcl._extract_ast_strings(vnode, var_table):
                    c = _canon_dep_label(raw, pkg_rel)
                    if c:
                        data_deps.add(_norm_vendor_crate_label(c))
    return code_deps, data_deps, overwrites, host_test_misplaced_data - data_deps


_GENERIC_TEST_DATA_SEGMENTS = {
    "test_data",
    "testdata",
    "data",
    "host_test_data",
    "keys",
    "repository",
    "blobs",
    "meta",
    "bin",
    "lib",
}
_pkg_source_blob_cache = {}
_htd_tokens_cache = {}


# Add non-generic staged directory path tokens for host_test_data consumption checks.
def _add_test_data_dir_tokens(tokens, raw_dir):
    dd = (raw_dir or "").strip().strip("/")
    if not dd:
        return
    tokens.add(dd)
    parent = os.path.dirname(dd)
    if parent and parent not in _GENERIC_TEST_DATA_SEGMENTS:
        tokens.add(parent)
    for seg in dd.split("/"):
        if seg and seg not in _GENERIC_TEST_DATA_SEGMENTS:
            tokens.add(seg)


# Read all non-BUILD source files in package `pkg_rel` to verify test_data consumption.
def _package_non_build_source_blob(wdir, pkg_rel):
    key = (wdir, pkg_rel)
    if key not in _pkg_source_blob_cache:
        chunks = []
        root = os.path.join(wdir, pkg_rel) if pkg_rel else wdir
        if os.path.isdir(root):
            for dirpath, dirnames, filenames in os.walk(root):
                dirnames[:] = [
                    dn
                    for dn in dirnames
                    if not dn.startswith(".")
                    and not os.path.isfile(os.path.join(dirpath, dn, "BUILD.bazel"))
                ]
                for fn in filenames:
                    if fn in ("BUILD.bazel", "BUILD.gn", "BUILD"):
                        continue
                    try:
                        with open(
                            os.path.join(dirpath, fn),
                            encoding="utf-8",
                            errors="ignore",
                        ) as sf:
                            chunks.append(sf.read())
                    except OSError:
                        pass
        _pkg_source_blob_cache[key] = "\n".join(chunks)
    return _pkg_source_blob_cache[key]


# Collect staged path tokens for a host_test_data target in BUILD.bazel or BUILD.gn.
def _host_test_data_target_tokens(wdir, base_rev, canon_lbl):
    if not canon_lbl or not canon_lbl.startswith("//") or not wdir:
        return None
    key = (wdir, base_rev, canon_lbl)
    if key in _htd_tokens_cache:
        return _htd_tokens_cache[key]
    dep_pkg, _, dep_name = canon_lbl[2:].partition(":")
    if not dep_pkg or not dep_name:
        _htd_tokens_cache[key] = None
        return None
    tokens = set()
    is_htd = False
    bz_path = os.path.join(wdir, dep_pkg, "BUILD.bazel")
    if os.path.isfile(bz_path):
        try:
            with open(bz_path, encoding="utf-8") as bf:
                bz_text = bf.read()
        except OSError:
            bz_text = ""
        if bz_text:
            var_table = _parse_bazel_var_table(bz_text, filename=bz_path)
            for bt in pcl.extract_bazel_targets(bz_text, filename=bz_path):
                if bt["name"] == dep_name and bt["rule"] in (
                    "host_test_data_files",
                    "host_test_data_map",
                ):
                    is_htd = True
                    kws = bt.get("keywords", {})
                    for dd in pcl._extract_ast_strings(
                        kws.get("dest_dir"), var_table
                    ):
                        _add_test_data_dir_tokens(tokens, dd)
                    for src in pcl._extract_ast_strings(kws.get("srcs"), var_table):
                        base = os.path.basename(src.split(":", 1)[-1]).strip()
                        if base and not tokens:
                            tokens.add(base)
    if not is_htd:
        gn_text = _read_dep_pkg_gn(wdir, dep_pkg, base_rev)
        for gt in pcl.extract_gn_targets(gn_text):
            if gt["name"] == dep_name and gt["tmpl"] == "host_test_data":
                is_htd = True
                body = gt["clean_body"]
                for om in re.finditer(r"(?<![\w.])outputs\s*\+?=\s*\[", body):
                    close_idx = pcl.find_closing_delim(body, om.end() - 1, "[", "]")
                    for raw_out in re.findall(
                        r'"((?:[^"\\]|\\.)*)"', body[om.end() : close_idx]
                    ):
                        cleaned = re.sub(
                            r"^\$\{?root_(?:out|build)_dir\}?/+", "", raw_out.strip()
                        )
                        cleaned = re.sub(r"/+\{\{[^}]+\}\}.*$", "", cleaned)
                        _add_test_data_dir_tokens(tokens, cleaned)
    res = tokens if (is_htd and tokens) else None
    _htd_tokens_cache[key] = res
    return res


# Check whether canon_lbl is a host_test_data target whose staged paths are unused in pkg_rel.
def _is_unused_host_test_data_in_pkg(wdir, base_rev, pkg_rel, canon_lbl):
    tokens = _host_test_data_target_tokens(wdir, base_rev, canon_lbl)
    if not tokens:
        return False
    src_blob = _package_non_build_source_blob(wdir, pkg_rel)
    return not any(tok in src_blob for tok in tokens)


# Check whether test_name was an explicit standalone `rustc_test("<test_name>")` in base
# BUILD.gn that omitted unused GN template `test_deps` from `<lib_name>`.
def _is_explicit_gn_rustc_test_omitting_template_deps(
    wdir, base_rev, pkg_rel, lib_name, test_name, gn_canon
):
    if not wdir or not pkg_rel or test_name == f"{lib_name}_test":
        return False
    base_gn = _read_base_pkg_gn(wdir, pkg_rel, base_rev)
    if not base_gn:
        return False
    blocks = [
        gt
        for gt in pcl.extract_gn_targets(base_gn)
        if gt["name"] == test_name and gt["tmpl"] == "rustc_test"
    ]
    if len(blocks) != 1:
        return False
    test_body_norms = {
        _norm_vendor_crate_label(c)
        for raw in re.findall(r'"((?:[^"\\]|\\.)*)"', blocks[0]["clean_body"])
        for c in (_canon_dep_label(raw, pkg_rel),)
        if c
    }
    return all(_norm_vendor_crate_label(g) not in test_body_norms for g in gn_canon)


# Check whether unit_test_parity ERROR/WARNING is a false positive on non-Rust
# runtime/compile test data (`data`/`test_data`/`compile_data`), mapped test deps,
# explicit standalone GN `rustc_test` targets, or unused GN `host_test_data` `test_deps`.
def is_verified_unit_test_parity_false_positive(finding, wdir="", base_rev=""):
    """Returns (test_name, kind) when reported GN test_deps are carried on test_name."""
    if finding.get("category") != "unit_test_parity":
        return None
    if finding.get("severity") not in ("ERROR", "WARNING"):
        return None
    rel_bz = finding.get("file", "")
    if not rel_bz.endswith("BUILD.bazel") or not wdir:
        return None
    m = UNIT_TEST_PARITY_MSG_RE.match(finding.get("message", ""))
    if not m:
        return None
    lib_name, raw_list, test_name = m.group(1), m.group(2), m.group(3)
    pkg_rel = os.path.dirname(rel_bz).strip("/")
    gn_items = [s.strip() for s in raw_list.split(",") if s.strip()]
    gn_canon = [_canon_dep_label(s, pkg_rel) for s in gn_items]
    if not gn_canon or None in gn_canon:
        return None
    ctx = _collect_unit_test_bazel_context(wdir, rel_bz, lib_name, test_name, pkg_rel)
    if ctx is None:
        return None
    code_deps, data_deps, overwrites, misplaced_data = ctx
    if any(_norm_vendor_crate_label(g) in misplaced_data for g in gn_canon):
        return (test_name, "data_not_test_data")
    if _is_explicit_gn_rustc_test_omitting_template_deps(
        wdir, base_rev, pkg_rel, lib_name, test_name, gn_canon
    ):
        return (test_name, "explicit_gn_rustc_test")
    stop_set = code_deps | data_deps
    used_runtime_data = False
    dropped_unused_test_data = False
    for g_lbl in gn_canon:
        g_norm = _norm_vendor_crate_label(g_lbl)
        if g_norm in code_deps:
            continue
        if g_norm in data_deps and not _is_bazel_rust_crate_target(wdir, g_lbl):
            used_runtime_data = True
            continue
        if g_norm in overwrites:
            ow_targets = overwrites[g_norm]
            if ow_targets & code_deps:
                continue
            ow_data = ow_targets & data_deps
            if ow_data and any(not _is_bazel_rust_crate_target(wdir, b) for b in ow_data):
                used_runtime_data = True
                continue
        if _is_unused_host_test_data_in_pkg(wdir, base_rev, pkg_rel, g_lbl):
            dropped_unused_test_data = True
            continue
        exp = _expand_pure_gn_forwarding_group(g_lbl, wdir, base_rev, stop_set)
        if not exp:
            return None
        for e_lbl in exp:
            if e_lbl in code_deps:
                continue
            if e_lbl in data_deps and not _is_bazel_rust_crate_target(wdir, e_lbl):
                used_runtime_data = True
                continue
            if _is_unused_host_test_data_in_pkg(wdir, base_rev, pkg_rel, e_lbl):
                dropped_unused_test_data = True
                continue
            return None
    if dropped_unused_test_data:
        return (test_name, "unused_host_test_data")
    return (test_name, "runtime_data" if used_runtime_data else "code_or_forwarding")


# Annotate verified review_diff false positives with accurate dispute instructions.
def annotate_review_diff_macro_false_positives(diff_findings, wdir, base_rev=""):
    """Updates remediation on verified false positives; returns (items, finding_ids)."""
    dispute_items = []
    seen_items = set()
    fp_ids = set()
    for f in diff_findings:
        f_path = f.get("file", "")
        f_cat = f.get("category", "")
        matched_attr = is_macro_synced_attr_false_positive(f, wdir, base_rev)
        matched_host = (
            None if matched_attr else is_host_only_macro_compat_false_positive(f, wdir)
        )
        matched_dep = (
            None
            if (matched_attr or matched_host)
            else is_verified_dep_alignment_false_positive(f, wdir, base_rev)
        )
        matched_unit_test = (
            None
            if (matched_attr or matched_host or matched_dep)
            else is_verified_unit_test_parity_false_positive(f, wdir, base_rev)
        )
        if matched_attr:
            tname, gn_attr, attr_kind = matched_attr
            if attr_kind == "local_gn_var":
                f["remediation"] = (
                    f'In `{f_path}`, `{gn_attr}` was a local GN helper variable '
                    "(not a standalone target parameter) whose value is already "
                    f'carried on "{tname}" (and preserved via `@bazel2gn` overwrite '
                    "when dual-built), so this `review_diff/attribute_alignment` "
                    "finding is a parser false positive that check scripts cannot "
                    "waive. Once all deterministic check scripts pass, dispute it in "
                    '`CoderReport.disputed_findings` (`source: "review_diff"`, '
                    f'`category: "attribute_alignment"`, `file: "{f_path}"`).'
                )
            else:
                f["remediation"] = (
                    f'Target "{tname}" already carries `{gn_attr}` in `{f_path}` '
                    "(and below `## BAZEL2GN SENTINEL` when dual-built), so this "
                    "`review_diff/attribute_alignment` WARNING is a Starlark macro "
                    "parser false positive that check scripts cannot waive. Once all "
                    "deterministic check scripts pass, dispute it in "
                    '`CoderReport.disputed_findings` (`source: "review_diff"`, '
                    f'`category: "attribute_alignment"`, `file: "{f_path}"`).'
                )
        elif matched_host:
            tname, rule_name = matched_host
            f["remediation"] = (
                f'Target "{tname}" uses host-only macro `{rule_name}`, which '
                "applies host platform constraints internally and does not take "
                "`target_compatible_with` (adding it breaks Bazel loading and "
                "`fx bazel2gn`). Do NOT add `target_compatible_with`; once all "
                "deterministic check scripts pass, dispute this `review_diff` "
                "false positive in `CoderReport.disputed_findings` "
                f'(`source: "review_diff"`, `category: "host_compatibility"`, '
                f'`file: "{f_path}"`).'
            )
        elif matched_dep:
            tname, dep_kind = matched_dep
            if dep_kind == "slash_or_shorthand":
                f["remediation"] = (
                    f'Target "{tname}" has identical dependencies in GN and Bazel '
                    "after normalizing duplicate slashes (`//` inside package paths, "
                    "which Bazel rejects) and shorthand target labels. Once all "
                    "deterministic check scripts pass, dispute this "
                    "`review_diff/dep_alignment` false positive in "
                    '`CoderReport.disputed_findings` (`source: "review_diff"`, '
                    f'`category: "dep_alignment"`, `file: "{f_path}"`).'
                )
            else:
                f["remediation"] = (
                    f'Target "{tname}" in `{f_path}` expands pure GN forwarding '
                    "group dependency target(s) directly to their underlying Bazel "
                    "target(s) (or maps them via `# @bazel2gn:path_overwrite:`), "
                    "which `gn_dep_parity` verifies and check scripts cannot waive "
                    "in `review_diff`. Once all deterministic check scripts pass, "
                    "dispute this `review_diff/dep_alignment` false positive in "
                    '`CoderReport.disputed_findings` (`source: "review_diff"`, '
                    f'`category: "dep_alignment"`, `file: "{f_path}"`).'
                )
        elif matched_unit_test and matched_unit_test[1] == "data_not_test_data":
            # Not a false positive: keep the finding blocking and point at `test_data`.
            f["remediation"] = (
                f'Target "{matched_unit_test[0]}" in `{f_path}` lists these runtime test data '
                "targets (e.g. `host_test_data_files`/`host_test_data_map`) in `data`, which "
                "only reaches the inner `rustc_test` binary. Move them to `test_data` of the "
                "`host_rustc_test` (forwarded to its `host_test()` wrapper). `bazel2gn` cannot "
                "translate `test_data` on `host_rustc_test`, so mark the test "
                "`# @bazel2gn:skip` (migration_sanity T3) with a comment, run it from Bazel via "
                "the nearest ancestor `test_suite` (`host_tests`), and remove the GN "
                "`host_test_data` targets above the sentinel if no GN consumers remain. Do NOT "
                "dispute this finding."
            )
            continue
        elif matched_unit_test:
            test_name, ut_kind = matched_unit_test
            if ut_kind == "unused_host_test_data":
                f["remediation"] = (
                    f'Target "{test_name}" in `{f_path}` intentionally omits unused '
                    "GN `host_test_data` `test_deps` entry/entries whose staged test "
                    "data paths are not referenced by any source or test file in the "
                    "package (adding them to `test_data` is rejected by "
                    "`test_data_attribute/unused_test_data`), which check scripts "
                    "cannot waive in `review_diff`. Once all deterministic check "
                    "scripts pass, dispute this `review_diff/unit_test_parity` false "
                    'positive in `CoderReport.disputed_findings` (`source: '
                    f'"review_diff"`, `category: "unit_test_parity"`, `file: '
                    f'"{f_path}"`).'
                )
            elif ut_kind == "runtime_data":
                f["remediation"] = (
                    f'Target "{test_name}" in `{f_path}` already carries these '
                    "non-Rust test dependencies in `test_data`/`compile_data` "
                    "(mapped to `non_rust_deps`/`inputs` by `bazel2gn`; Rust `deps` "
                    "requires `CrateInfo` and rejects runtime data targets). Do NOT "
                    "move them to `deps`; once all deterministic check scripts pass, "
                    "dispute this `review_diff/unit_test_parity` false positive in "
                    '`CoderReport.disputed_findings` (`source: "review_diff"`, '
                    f'`category: "unit_test_parity"`, `file: "{f_path}"`).'
                )
            else:
                f["remediation"] = (
                    f'Target "{test_name}" in `{f_path}` already carries these GN '
                    "`test_deps` entries (via normalized Bazel labels, `link_deps`, "
                    "pure GN forwarding group expansion, or "
                    "`# @bazel2gn:path_overwrite:`), which check scripts cannot "
                    "waive in `review_diff`. Once all deterministic check scripts "
                    "pass, dispute this `review_diff/unit_test_parity` false "
                    'positive in `CoderReport.disputed_findings` (`source: '
                    f'"review_diff"`, `category: "unit_test_parity"`, `file: '
                    f'"{f_path}"`).'
                )
        else:
            continue
        fp_ids.add(id(f))
        item_key = (f_cat, f_path)
        if f_cat and f_path and item_key not in seen_items:
            seen_items.add(item_key)
            dispute_items.append(item_key)
    return dispute_items, fp_ids


def precompute_review_diff():
    global _precomputed_diff_findings
    planter_bin = rl.find_planter_bin()
    if not planter_bin or not workdir:
        return
    env = dict(os.environ, PLANTER_BIN=planter_bin, PLANTER_WORKDIR=workdir,
               PLANTER_TARGET_DIRS=" ".join(target_dirs), PLANTER_CHANGE_BASE=change_base,
               PLANTER_RESOLVED_CHANGE_BASE=change_base, PLANTER_SKIP_FETCH="1",
               PLANTER_SKIP_DIFF_COVERAGE="1")
    for flag in ("--format=findings-json", "--check-findings"):
        out, _, err_text = execute([planter_bin, "diff", "--local", flag, f"--workdir={workdir}"], workdir, env, "review_diff")
        findings, parsed_ok = pcl.parse_check_findings(out.strip())
        if parsed_ok or (err_text is None and out.strip().startswith("[")):
            _precomputed_diff_findings = findings
            break
    if _precomputed_diff_findings is None:
        _precomputed_diff_findings = []
    try:
        tf = tempfile.NamedTemporaryFile("w", prefix="planter-review-diff-", suffix=".json", delete=False)
        json.dump(_precomputed_diff_findings, tf)
        tf.close()
        os.environ["PLANTER_REVIEW_DIFF_JSON"] = tf.name
        atexit.register(lambda p=tf.name: os.path.exists(p) and os.unlink(p))
    except OSError:
        pass


def run_once(spec, target_dir):
    script = spec["script_file"]
    if not os.path.isabs(script):
        for cand in (
            os.path.normpath(os.path.join(mroot, "checks", script)),
            os.path.normpath(os.path.join(project_dir, "checks", script)),
            os.path.normpath(os.path.join(mroot, script)),
            os.path.normpath(os.path.join(project_dir, script)),
        ):
            if os.path.exists(cand):
                script = cand
                break
        else:
            return [pcl.new_finding(
                source=spec["name"], category="check_script_missing_in_pin", severity="INFO",
                message=f"check script {json.dumps(spec['script_file'])} is not present in this pinned machinery snapshot; skipping.",
            )]
    path = os.environ.get("PATH", "")
    env = dict(os.environ, SMELT_PROJECT_DIR=project_dir, PLANTER_PROJECT_DIR=project_dir,
               PLANTER_WORKDIR=workdir, PLANTER_CHECK_NAME=spec["name"],
               PLANTER_TARGET_DIR=target_dir, PLANTER_TARGET_DIRS=" ".join(target_dirs),
               PLANTER_CHANGE_BASE=change_base, PLANTER_RESOLVED_CHANGE_BASE=change_base,
               PLANTER_SKIP_FETCH="1",
               PATH=os.path.join(project_dir, "tools") + (os.pathsep + path if path else ""))
    if skip_build:
        env["PLANTER_SKIP_BUILD"] = "1"
    label = spec["name"] + (f" ({target_dir})" if len(target_dirs) > 1 else "")
    out, err, err_text = execute(["bash", script] + spec["args"], workdir or project_dir, env, label)
    raw = out.strip()
    findings, parsed_ok = pcl.parse_check_findings(raw)
    for f in findings:
        f["source"] = f["source"] or spec["name"]
        f["severity"] = pcl.normalize_severity(f["severity"])
    if err_text is not None and (not parsed_ok or not any(pcl.is_blocking_finding(f) for f in findings)):
        findings.append(pcl.new_finding(
            source=spec["name"], category="check_script_failure", severity="ERROR",
            message=f"check tool {json.dumps(spec['name'], ensure_ascii=False)} exited with error ({err_text}): {err.strip() or raw or err_text}",
        ))
    return findings


def _exec_check(spec):
    started, seen, results = time.monotonic(), set(), []
    for d in target_dirs or [""]:
        for f in run_once(spec, d):
            key = (f["source"], f["category"], f["file"], f["line"], f["message"])
            if key not in seen:
                seen.add(key)
                if len(target_dirs) > 1 and not f["file"] and d:
                    f["message"] = f"[{d}] {f['message']}"
                results.append(f)
    return spec, results, time.monotonic() - started


builds_skipped = skip_build or os.environ.get("PLANTER_SKIP_BUILD", "").strip() not in ("", "0")
log(f"{len(enabled)} check(s) in {workdir}; target dirs: {' '.join(target_dirs) or '(none)'}; change base: {change_base}" + ("; builds skipped" if builds_skipped else ""))
max_workers = 1 if os.environ.get("PLANTER_SERIAL_CHECKS", "").strip() in ("1", "true") else min(8, os.cpu_count() or 4)

# Static checks run in parallel; build_verification and tests_json_parity NEVER run concurrently
# with static checks or with each other so tests_json_parity's tree/args.gn swap never races fx build.
static_specs = [s for s in enabled if s["name"] not in SERIAL_BUILD_CHECKS]
serial_specs = [s for s in enabled if s["name"] in SERIAL_BUILD_CHECKS]

try:
    diff_start = time.monotonic()
    if not only or "review_diff" in only or any(s["name"] in ("visibility_audit", "migration_sanity") for s in enabled):
        precompute_review_diff()
    diff_duration = time.monotonic() - diff_start

    total, idx_map, results_by_idx, done_count = len(enabled), {id(s): i for i, s in enumerate(enabled)}, {}, 0
    if static_specs:
        with ThreadPoolExecutor(max_workers=max_workers) as executor:
            for fut in [executor.submit(_exec_check, spec) for spec in static_specs]:
                spec, findings, duration = fut.result()
                done_count += 1
                log(f"({done_count}/{total}) {spec['name']} done ({pcl.fmt_secs(duration)})")
                results_by_idx[idx_map[id(spec)]] = (spec, findings, duration)
    for spec in serial_specs:
        spec, findings, duration = _exec_check(spec)
        done_count += 1
        log(f"({done_count}/{total}) {spec['name']} done ({pcl.fmt_secs(duration)})")
        results_by_idx[idx_map[id(spec)]] = (spec, findings, duration)

    rows = [results_by_idx[i] for i in range(total)]
    all_findings = [f for _, flist, _ in rows for f in flist]

    if (not only or "review_diff" in only) and _precomputed_diff_findings is not None:
        # Same rule as planter's upload gate (mergeReviewDiffFindings): a review_diff finding is
        # skipped only when a check reported the same key at a severity that blocks at least as
        # much, or explicitly waived it in a waivable category. A non-blocking check finding never
        # hides a blocking review_diff finding.
        seen_blocking = {}
        waived = set()
        for f in all_findings:
            key = (f["category"], f["file"], f["line"], f["message"])
            seen_blocking[key] = seen_blocking.get(key, False) or pcl.is_blocking_finding(f)
            if f.get("waiver") and f["category"] in WAIVABLE_REVIEW_DIFF_CATEGORIES:
                waived.add(key)
        diff_findings = []
        for raw_f in _precomputed_diff_findings:
            f = dict(raw_f, source="review_diff", severity=pcl.normalize_severity(raw_f.get("severity", "")))
            key = (f["category"], f["file"], f["line"], f["message"])
            blocks = pcl.is_blocking_finding(f)
            if key in waived or (key in seen_blocking and (seen_blocking[key] or not blocks)):
                continue
            seen_blocking[key] = blocks
            diff_findings.append(f)
        macro_dispute_items, verified_diff_fp_ids = (
            annotate_review_diff_macro_false_positives(
                diff_findings, workdir, change_base
            )
        )
        rows.append(({"name": "review_diff", "enabled": True}, diff_findings, diff_duration))
        all_findings.extend(diff_findings)
except KeyboardInterrupt:
    log("interrupted")
    sys.exit(130)

# Downgrade unamendable commit-message findings when a workspace write error occurred,
# or when build_verification built the `test` host tool during the same run.
if any(f["category"] == "workspace_write_error" for f in all_findings):
    for f in all_findings:
        if f["source"] == "commit_message_format" and pcl.is_blocking_finding(f):
            f["severity"] = "INFO"
if not builds_skipped and not any(f["category"] == "unbuilt_fx_test_tool" for f in all_findings):
    for f in all_findings:
        if (
            f["source"] == "commit_message_format"
            and f["category"] == "test_footer_fx_test_tool_not_built"
        ):
            f["severity"] = "INFO"

blocking_findings = [f for f in all_findings if pcl.is_blocking_finding(f)]
exit_code = 1 if blocking_findings else 0

if as_json:
    out = [
        {k: f[k] for k in ("source", "category", "severity", "file", "line", "message", "remediation", "waiver") if k in ("source", "category", "severity", "message") or f.get(k)}
        for f in all_findings
    ]
    print(json.dumps(out, indent=2))
    sys.exit(exit_code)


def counts(findings):
    return ", ".join(f"{n} {sev}" for sev in ("ERROR", "WARNING", "INFO") for n in [sum(1 for f in findings if f["severity"] == sev)] if n)


def print_finding(prefix, f):
    where = f["file"] + (f":{f['line']}" if f["file"] and f["line"] else "")
    print(f"{prefix}[{f['severity']}] {f['source']}/{f['category'] or '-'}" + (f"  {where}" if where else ""))
    for line in f["message"].splitlines() or [""]:
        print("    " + line)
    if f["remediation"]:
        rem = f["remediation"].splitlines()
        print("    Remediation: " + rem[0])
        for line in rem[1:]:
            print("    " + line)


width = max([len(s["name"]) for s, _, _ in rows] + [len(s["name"]) for s in disabled] + [4])
print("Checks (planter runs these, in this order, after you report):")
for spec, findings, secs in rows:
    status = "FAIL" if any(pcl.is_blocking_finding(f) for f in findings) else "PASS"
    detail = counts(findings)
    print(f"  {status}  {spec['name']:<{width}}  {pcl.fmt_secs(secs):>7}" + (f"  {detail}" if detail else ""))
for spec in disabled:
    print(f"  ----  {spec['name']:<{width}}  disabled in checks/manifest.json (planter skips it)")
if not rows:
    print("  (no enabled checks)")

if blocking_findings:
    print(f"\nBlocking findings ({len(blocking_findings)}); planter sends the change back for any of these:\n")
    for n, f in enumerate(blocking_findings, 1):
        print_finding(f"{n}. ", f)
        print()
others = [f for f in all_findings if not pcl.is_blocking_finding(f)]
if others:
    print(f"\nNon-blocking findings ({len(others)}):\n")
    for f in others:
        print_finding("- ", f)
        print()

print()
if blocking_findings:
    sources = sorted({f["source"] for f in blocking_findings})
    print(f"RESULT: FAIL: {len(blocking_findings)} blocking finding(s) ({counts(blocking_findings)}) from {', '.join(sources)}.")
    # Guide CoderReport.disputed_findings when all check scripts passed and only
    # review_diff WARNINGs or verified review_diff false positives remain.
    if all(
        f["source"] == "review_diff"
        and (f["severity"] == "WARNING" or id(f) in verified_diff_fp_ids)
        for f in blocking_findings
    ):
        if macro_dispute_items:
            items_str = ", ".join(
                f"`review_diff/{cat}` (`{p}`)" for cat, p in macro_dispute_items
            )
            print(
                "NOTE: all deterministic check scripts passed; only `review_diff` "
                "finding(s) remain (including verified macro/forwarding-group/label "
                "false positive(s)). Undisputed blocking `review_diff` findings "
                "still reject the round: fix any genuine parity gap in `BUILD.bazel`, "
                f"or list EVERY verified false positive ({items_str}) in "
                "`CoderReport.disputed_findings` instead of `[]`."
            )
        else:
            print(
                "NOTE: all deterministic check scripts passed; only `review_diff` "
                "WARNING finding(s) remain. Undisputed `review_diff` WARNINGs block "
                "upload: fix genuine parity gaps in `BUILD.bazel`, or dispute verified "
                "`review_diff` false positives for every affected file in "
                "`CoderReport.disputed_findings`."
            )
else:
    # Distinguish one-turn mid-turn pin lag from persistent host/container check skew.
    print(
        "RESULT: PASS: no blocking findings in live `$PLANTER_PROJECT_DIR` checks. "
        "Set `disputed_findings` to `[]` in `CoderReport` (including once if a mid-turn "
        "machinery update missed the prior round's start-of-turn check snapshot). Only if "
        "the exact same post-turn check finding absent from `run_checks.sh` repeats across "
        "consecutive clean rounds with no mid-turn update, report that persistent check-"
        "runner skew in `disputed_findings` so the stale pin or runner is refreshed."
    )
if any(f["category"] == "workspace_write_error" for f in blocking_findings):
    print(
        "NOTE: workspace filesystem/mount write failure detected (`workspace_write_error`). "
        "Stop retrying build or `git commit` write commands, set `test_footers` to "
        "`[\"fx host-tool shac check\"]` and `disputed_findings` to `[]` in `CoderReport` "
        "(empty `test_footers` fails `test_verifier` with `missing_test_footer`, while "
        "target-bearing `Test:` commands trigger spurious query errors on an unwritable "
        "mount), and report the mount write failure in `summary`."
    )
if only:
    print("NOTE: only some checks ran (--only). Run all of them before reporting.")
if any(f["category"] == "build_skipped" for f in all_findings):
    print("NOTE: the builds were skipped. Run `run_checks.sh` without --skip-build before reporting.")
sys.exit(exit_code)
PYEOF
