#!/usr/bin/env fuchsia-vendored-python
# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

"""Unit tests for planter_check_lib (and rdeps_lib)."""

import io
import json
import os
import subprocess
import sys
import tempfile
import textwrap
import unittest
from contextlib import redirect_stdout
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import planter_check_lib as pcl


class PlanterCheckLibTest(unittest.TestCase):
    def test_canonical_sets(self):
        self.assertIn("bundles/assembly", pcl.EXCLUDED_GLOBAL_DIRS)
        self.assertIn("third_party/rust_crates", pcl.EXCLUDED_GLOBAL_DIRS)
        self.assertIn("rustc_test", pcl.BAZEL_TEST_RULES)
        self.assertIn("fx_unittest_package", pcl.BAZEL_TEST_RULES)
        self.assertIn("template", pcl.GN_NON_TARGET_KEYWORDS)
        self.assertIn("declare_args", pcl.GN_NON_TARGET_KEYWORDS)
        self.assertEqual(pcl.NOT_TARGETS, pcl.GN_NON_TARGET_KEYWORDS)

    def test_emit_and_emit_json(self):
        findings = []
        with mock.patch.dict(os.environ, {"PLANTER_CHECK_NAME": "my_check"}):
            entry = pcl.emit(
                findings,
                category="test_cat",
                severity="warn",
                file="src/foo/BUILD.bazel",
                line=42,
                message="something is off",
                remediation="fix it",
            )
        self.assertEqual(len(findings), 1)
        self.assertEqual(entry["source"], "my_check")
        self.assertEqual(entry["category"], "test_cat")
        self.assertEqual(entry["severity"], "WARNING")
        self.assertEqual(entry["file"], "src/foo/BUILD.bazel")
        self.assertEqual(entry["line"], 42)

        pcl.emit(
            findings,
            "info_cat",
            "nit",
            "a/b",
            1,
            "m",
            "r",
            source="explicit_src",
        )
        self.assertEqual(findings[1]["source"], "explicit_src")
        self.assertEqual(findings[1]["severity"], "INFO")

        pcl.emit(findings, "err_cat", "error", "a/b", 2, "m", "r")
        self.assertEqual(findings[2]["severity"], "error")

        buf = io.StringIO()
        with redirect_stdout(buf):
            pcl.emit_json(findings)
        parsed = json.loads(buf.getvalue())
        self.assertEqual(len(parsed), 3)
        self.assertEqual(parsed[0]["category"], "test_cat")

    def test_normalize_target_dir(self):
        self.assertEqual(
            pcl.normalize_target_dir("//src/lib/foo/"), "src/lib/foo"
        )
        self.assertEqual(
            pcl.normalize_target_dir("./src/lib/foo"), "src/lib/foo"
        )
        self.assertEqual(pcl.normalize_target_dir("."), "")
        self.assertEqual(
            pcl.normalize_target_dir(
                "/work/tree/src/bar", workdir="/work/tree"
            ),
            "src/bar",
        )

    def test_strip_and_blank_comments(self):
        src = textwrap.dedent(
            """            # Top comment
            rustc_library("foo") { # inline comment
              name = "foo#not_a_comment"
              deps = [ "//a/b" ] # trailing
            }
        """
        )
        stripped = pcl.strip_comments(src)
        self.assertNotIn("Top comment", stripped)
        self.assertNotIn("inline comment", stripped)
        self.assertIn('"foo#not_a_comment"', stripped)
        self.assertEqual(stripped.count("\n"), src.count("\n"))

        blanked = pcl.blank_comments(src)
        self.assertEqual(len(blanked), len(src))
        self.assertEqual(blanked.count("\n"), src.count("\n"))
        self.assertIn('"foo#not_a_comment"', blanked)

    def test_split_sentinel(self):
        gn_with_sentinel = textwrap.dedent(
            """            import("//build/rust/rustc_library.gni")
            group("tests") {
              testonly = true
            }
            ## BAZEL2GN SENTINEL - DO NOT EDIT BELOW THIS LINE ##
            rustc_library("foo") {
              edition = "2021"
            }
        """
        )
        self.assertTrue(pcl.has_sentinel(gn_with_sentinel))
        above, below = pcl.split_sentinel(gn_with_sentinel)
        self.assertIn('group("tests")', above)
        self.assertNotIn('rustc_library("foo")', above)
        self.assertIn('rustc_library("foo")', below)

        gn_local_sentinel = (
            'group("a") {}\n#LOCAL_BAZEL_BUILD_SENTINEL\nsource_set("b") {}\n'
        )
        self.assertTrue(pcl.has_sentinel(gn_local_sentinel))
        above2, below2 = pcl.split_sentinel(gn_local_sentinel)
        self.assertIn('group("a")', above2)
        self.assertIn('source_set("b")', below2)

        gn_no_sentinel = 'group("only") {}\n'
        self.assertFalse(pcl.has_sentinel(gn_no_sentinel))
        above3, below3 = pcl.split_sentinel(gn_no_sentinel)
        self.assertEqual(above3, gn_no_sentinel)
        self.assertEqual(below3, "")

    def test_extract_gn_targets(self):
        gn_text = textwrap.dedent(
            """            import("//build/components.gni")
            # rustc_library("commented_out") {}
            template("my_tmpl") {
              source_set(target_name) {}
            }
            if (is_host) {
              rustc_binary("my_bin") {
                deps = [ ":lib" ]
              }
            }
            rustc_library("lib") {
              sources = [ "src/lib.rs" ]
            }
        """
        )
        targets = pcl.extract_gn_targets(gn_text)
        names = [(t["type"], t["name"], t["line"]) for t in targets]
        self.assertEqual(
            names,
            [
                ("rustc_binary", "my_bin", 7),
                ("rustc_library", "lib", 11),
            ],
        )
        with_tmpls = pcl.extract_gn_targets(gn_text, include_templates=True)
        self.assertEqual(len(with_tmpls), 3)
        self.assertEqual(with_tmpls[0]["name"], "my_tmpl")

    def test_extract_bazel_targets(self):
        bazel_text = textwrap.dedent(
            """            load("//build/bazel/rules/rust:defs.bzl", "rustc_library", "rustc_test")

            COMMON_VIS = ["//src/sys:__subpackages__"]

            rustc_library(
                name = "foo",
                srcs = ["src/lib.rs"],
                deps = ["//src/lib/bar"],
                visibility = COMMON_VIS + ["//src/diagnostics:__pkg__"],
            )

            # @bazel2gn:skip
            rustc_test(
                name = "foo_test",
                testonly = True,
                deps = [":foo"],
            )
        """
        )
        targets = pcl.extract_bazel_targets(bazel_text)
        self.assertEqual(len(targets), 2)
        t0, t1 = targets[0], targets[1]
        self.assertEqual(t0["name"], "foo")
        self.assertEqual(t0["rule"], "rustc_library")
        self.assertFalse(t0["has_bazel2gn_skip"])
        self.assertEqual(
            t0["visibility"],
            ["//src/sys:__subpackages__", "//src/diagnostics:__pkg__"],
        )
        self.assertEqual(t0["deps"], ["//src/lib/bar"])

        self.assertEqual(t1["name"], "foo_test")
        self.assertEqual(t1["rule"], "rustc_test")
        self.assertTrue(t1["testonly"])
        self.assertTrue(t1["has_bazel2gn_skip"])

    def test_read_review_diff_findings(self):
        with tempfile.NamedTemporaryFile(
            "w", suffix=".json", delete=False
        ) as tf:
            json.dump([{"category": "visibility_narrowed", "line": 10}], tf)
            path = tf.name
        try:
            with mock.patch.dict(
                os.environ, {"PLANTER_REVIEW_DIFF_JSON": path}
            ):
                res = pcl.read_review_diff_findings()
                self.assertEqual(len(res), 1)
                self.assertEqual(res[0]["category"], "visibility_narrowed")
            with mock.patch.dict(
                os.environ, {"PLANTER_REVIEW_DIFF_JSON": "/nonexistent"}
            ):
                self.assertEqual(pcl.read_review_diff_findings(), [])
        finally:
            os.unlink(path)

    def test_resolve_change_base_and_changed_files(self):
        with mock.patch.dict(
            os.environ,
            {
                "PLANTER_RESOLVED_CHANGE_BASE": "deadbeef",
                "PLANTER_CHANGED_FILES": "src/a/BUILD.bazel\nsrc/a/BUILD.gn\n",
            },
        ):
            self.assertEqual(pcl.resolve_change_base("/tmp"), "deadbeef")
            self.assertEqual(
                pcl.changed_files("/tmp"),
                ["src/a/BUILD.bazel", "src/a/BUILD.gn"],
            )

        with tempfile.TemporaryDirectory() as tmpdir:
            subprocess.run(["git", "-C", tmpdir, "init", "-q"], check=True)
            subprocess.run(
                [
                    "git",
                    "-C",
                    tmpdir,
                    "config",
                    "user.email",
                    "test@example.com",
                ],
                check=True,
            )
            subprocess.run(
                ["git", "-C", tmpdir, "config", "user.name", "Test"],
                check=True,
            )
            f1 = os.path.join(tmpdir, "BUILD.gn")
            with open(f1, "w") as f:
                f.write("# init\n")
            subprocess.run(["git", "-C", tmpdir, "add", "BUILD.gn"], check=True)
            subprocess.run(
                ["git", "-C", tmpdir, "commit", "-q", "-m", "initial"],
                check=True,
            )

            f2 = os.path.join(tmpdir, "BUILD.bazel")
            with open(f2, "w") as f:
                f.write("# migrated\n")
            subprocess.run(
                ["git", "-C", tmpdir, "add", "BUILD.bazel"], check=True
            )
            subprocess.run(
                [
                    "git",
                    "-C",
                    tmpdir,
                    "commit",
                    "-q",
                    "-m",
                    "[bazel2gn] Migrate pkg",
                ],
                check=True,
            )
            with open(os.path.join(tmpdir, "CoderReport1.json"), "w") as f:
                f.write("{}")

            env = {
                k: v
                for k, v in os.environ.items()
                if k
                not in (
                    "PLANTER_RESOLVED_CHANGE_BASE",
                    "PLANTER_CHANGED_FILES",
                    "PLANTER_CHANGE_BASE",
                )
            }
            with mock.patch.dict(os.environ, env, clear=True):
                self.assertEqual(pcl.resolve_change_base(tmpdir), "HEAD~1")
                cfiles = pcl.changed_files(tmpdir)
                self.assertIn("BUILD.bazel", cfiles)
                self.assertNotIn("CoderReport1.json", cfiles)


import rdeps_lib as rl


class RdepsLibAndScriptsTest(unittest.TestCase):
    def test_visibility_recommendation_tiers(self):
        # Tier 1: narrow
        self.assertEqual(
            rl.recommend_narrow_visibility(
                ["src/sys/pkg/bin/pkg-cache", "src/sys/pkg/lib/fuchsia-pkg"],
                "src/lib/my_lib",
            ),
            [
                "//src/sys/pkg/bin/pkg-cache:__pkg__",
                "//src/sys/pkg/lib/fuchsia-pkg:__pkg__",
            ],
        )
        # Vendor google sanitization
        self.assertEqual(
            rl.recommend_narrow_visibility(
                ["vendor/google/foo/bar", "src/diagnostics/archivist"],
                "src/lib/my_lib",
            ),
            [
                "//src/diagnostics/archivist:__pkg__",
                "//vendor/google:__subpackages__",
            ],
        )

        # Tier 2: area rollup when narrow > 8
        many_callers = [f"src/connectivity/sub_{i}/leaf" for i in range(5)] + [
            f"src/diagnostics/tool_{i}" for i in range(5)
        ]
        self.assertEqual(
            rl.recommend_visibility(many_callers, "src/lib/my_lib"),
            [
                "//src/connectivity:__subpackages__",
                "//src/diagnostics:__subpackages__",
            ],
        )

        # Tier 3: globally used across >= 6 areas and >= 20 callers
        global_callers = [
            f"src/{area}/comp_{i}"
            for area in (
                "connectivity",
                "diagnostics",
                "devices",
                "Modular",
                "security",
                "storage",
                "sys",
            )
            for i in range(8)
        ]
        self.assertTrue(
            rl.is_globally_used_target(global_callers, "src/lib/base")
        )

        # Multi-area test/benchmark callers -> //:__subpackages__
        test_callers = [
            "src/sys/pkg/tests/pkg-resolver",
            "src/storage/benchmarks/vfs",
        ]
        self.assertTrue(
            rl.has_only_test_or_bench_rdeps(test_callers, "src/lib/helper")
        )
        self.assertEqual(
            rl.recommend_visibility(
                test_callers, "src/lib/helper", target_name="helper"
            ),
            ["//:__subpackages__"],
        )

    def test_find_rdeps_and_visibility_audit_integration(self):
        checks_dir = os.path.dirname(os.path.abspath(__file__))
        tools_dir = os.path.abspath(os.path.join(checks_dir, "..", "tools"))
        find_rdeps_sh = os.path.join(tools_dir, "find_rdeps.sh")
        vis_audit_sh = os.path.join(checks_dir, "visibility_audit.sh")

        with tempfile.TemporaryDirectory() as tmpdir:
            subprocess.run(["git", "-C", tmpdir, "init", "-q"], check=True)
            subprocess.run(
                ["git", "-C", tmpdir, "config", "user.email", "t@example.com"],
                check=True,
            )
            subprocess.run(
                ["git", "-C", tmpdir, "config", "user.name", "Test"],
                check=True,
            )

            lib_dir = os.path.join(tmpdir, "src/lib/foo")
            caller_dir = os.path.join(tmpdir, "src/sys/bar")
            os.makedirs(lib_dir)
            os.makedirs(caller_dir)

            with open(os.path.join(lib_dir, "BUILD.gn"), "w") as f:
                f.write(
                    """
rustc_library("foo") {
  sources = [ "src/lib.rs" ]
}
group("tests") {
  testonly = true
}
"""
                )
            with open(os.path.join(caller_dir, "BUILD.gn"), "w") as f:
                f.write(
                    """
rustc_binary("bar") {
  deps = [ "//src/lib/foo:foo" ]
}
"""
                )
            subprocess.run(["git", "-C", tmpdir, "add", "."], check=True)
            subprocess.run(
                ["git", "-C", tmpdir, "commit", "-q", "-m", "init"], check=True
            )

            # Now migrate src/lib/foo to BUILD.bazel without visibility first
            with open(os.path.join(lib_dir, "BUILD.bazel"), "w") as f:
                f.write(
                    """
load("//build/bazel/rules/rust:defs.bzl", "rustc_library")

rustc_library(
    name = "foo",
    srcs = ["src/lib.rs"],
)
"""
                )
            env = dict(os.environ)
            env["PLANTER_WORKDIR"] = tmpdir
            env["PLANTER_TARGET_DIR"] = "src/lib/foo"
            env["PLANTER_SKIP_FETCH"] = "1"
            env["PLANTER_SKIP_DIFF_COVERAGE"] = "1"
            env.pop("PLANTER_CHANGED_FILES", None)
            env.pop("PLANTER_RESOLVED_CHANGE_BASE", None)

            rdeps_out = subprocess.check_output(
                [find_rdeps_sh, "src/lib/foo"], env=env, text=True
            )
            rdeps_data = json.loads(rdeps_out)
            self.assertEqual(
                rdeps_data["true_rdep_packages"], ["//src/sys/bar"]
            )
            self.assertEqual(
                rdeps_data["per_target_recommended_visibility"]["foo"],
                ["//src/sys/bar:__pkg__"],
            )

            # visibility_audit should report missing_target_visibility
            va_out = subprocess.check_output([vis_audit_sh], env=env, text=True)
            va_findings = json.loads(va_out)
            self.assertEqual(len(va_findings), 1)
            self.assertEqual(
                va_findings[0]["category"], "missing_target_visibility"
            )

            # Now set the recommended visibility on foo
            with open(os.path.join(lib_dir, "BUILD.bazel"), "w") as f:
                f.write(
                    """
load("//build/bazel/rules/rust:defs.bzl", "rustc_library")

rustc_library(
    name = "foo",
    srcs = ["src/lib.rs"],
    visibility = ["//src/sys/bar:__pkg__"],
)
"""
                )
            va_out_clean = subprocess.check_output(
                [vis_audit_sh], env=env, text=True
            )
            self.assertEqual(json.loads(va_out_clean), [])

    def test_manifest_and_findings_parsing(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            mpath = os.path.join(tmpdir, "manifest.json")
            with open(mpath, "w") as f:
                json.dump(
                    {
                        "version": 1,
                        "checks": [
                            {
                                "name": "vis_check",
                                "description": "Visibility check",
                                "script_file": "visibility_audit.sh",
                                "args": ["--strict"],
                                "enabled": True,
                            }
                        ],
                    },
                    f,
                )
            specs = pcl.load_check_manifest(mpath)
            self.assertEqual(len(specs), 1)
            self.assertEqual(specs[0]["name"], "vis_check")
            self.assertTrue(specs[0]["enabled"])

            # Recovery of trailing JSON array after stdout noise
            noisy_raw = 'some debug log\n[{"category": "cat1", "severity": "warn", "line": 7, "message": "m"}]'
            findings, ok = pcl.parse_check_findings(noisy_raw)
            self.assertTrue(ok)
            self.assertEqual(len(findings), 1)
            self.assertEqual(findings[0]["category"], "cat1")
            self.assertEqual(findings[0]["line"], 7)

    def test_run_checks_integration(self):
        run_checks_sh = os.path.normpath(
            os.path.join(
                os.path.dirname(os.path.abspath(__file__)),
                "..",
                "tools",
                "run_checks.sh",
            )
        )
        list_out = subprocess.check_output([run_checks_sh, "--list"], text=True)
        self.assertIn("visibility_audit:", list_out)
        self.assertIn("migration_sanity:", list_out)

        with tempfile.TemporaryDirectory() as tmpdir:
            subprocess.run(["git", "-C", tmpdir, "init", "-q"], check=True)
            subprocess.run(
                [
                    "git",
                    "-C",
                    tmpdir,
                    "config",
                    "user.email",
                    "test@example.com",
                ],
                check=True,
            )
            subprocess.run(
                ["git", "-C", tmpdir, "config", "user.name", "Test User"],
                check=True,
            )
            pkg_dir = os.path.join(tmpdir, "src", "lib", "foo")
            os.makedirs(pkg_dir)
            with open(os.path.join(pkg_dir, "BUILD.gn"), "w") as f:
                f.write('rustc_library("foo") {}\n')
            subprocess.run(["git", "-C", tmpdir, "add", "."], check=True)
            subprocess.run(
                ["git", "-C", tmpdir, "commit", "-q", "-m", "init"], check=True
            )

            env = dict(os.environ)
            env["PLANTER_WORKDIR"] = tmpdir
            env["PLANTER_SKIP_FETCH"] = "1"
            env.pop("PLANTER_CHANGED_FILES", None)
            env.pop("PLANTER_RESOLVED_CHANGE_BASE", None)

            proc = subprocess.run(
                [
                    run_checks_sh,
                    "--only=visibility_audit",
                    "--json",
                    "src/lib/foo",
                ],
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(proc.returncode, 0, msg=proc.stderr)
            self.assertEqual(json.loads(proc.stdout), [])


if __name__ == "__main__":
    unittest.main()
