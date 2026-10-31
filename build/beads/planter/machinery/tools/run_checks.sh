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
import atexit
from concurrent.futures import ThreadPoolExecutor
import json
import os
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
        rows.append(({"name": "review_diff", "enabled": True}, diff_findings, diff_duration))
        all_findings.extend(diff_findings)
except KeyboardInterrupt:
    log("interrupted")
    sys.exit(130)

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
else:
    print("RESULT: PASS: no blocking findings (set disputed_findings to [] in CoderReport; any entry triggers machinery_conflict).")
if only:
    print("NOTE: only some checks ran (--only). Run all of them before reporting.")
if any(f["category"] == "build_skipped" for f in all_findings):
    print("NOTE: the builds were skipped. Run `run_checks.sh` without --skip-build before reporting.")
sys.exit(exit_code)
PYEOF
