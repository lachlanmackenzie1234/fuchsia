# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

import contextlib
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import registry


def _find_tool_launcher() -> list[str] | None:
    this_dir = Path(__file__).resolve().parent
    if (this_dir / "registry.py").is_file():
        return [sys.executable, str(this_dir / "registry.py")]
    rel = "tools/devshell/fx_cli/registry"
    candidates = [
        Path(rel),
        Path("_main") / rel,
    ]
    runfiles_dir = os.environ.get("RUNFILES_DIR")
    if runfiles_dir:
        candidates.insert(0, Path(runfiles_dir) / "_main" / rel)
    for candidate in candidates:
        if (
            candidate.is_file()
            and os.access(candidate, os.X_OK)
            and not candidate.name.endswith(".py")
        ):
            return [str(candidate)]
        py_cand = candidate.with_suffix(".py")
        if py_cand.is_file():
            return [sys.executable, str(py_cand)]
    return None


class FxRegistryToolTests(unittest.TestCase):
    def _run_registry(
        self,
        args: list[str],
        fuchsia_dir: Path | None = None,
    ) -> subprocess.CompletedProcess[str]:
        launcher_cmd = _find_tool_launcher()
        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        }
        if fuchsia_dir is not None:
            env["FUCHSIA_DIR"] = str(fuchsia_dir)
        if launcher_cmd is not None:
            return subprocess.run(
                launcher_cmd + args,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

        out_buf = io.StringIO()
        err_buf = io.StringIO()
        with (
            mock.patch.dict(os.environ, env, clear=True),
            contextlib.redirect_stdout(out_buf),
            contextlib.redirect_stderr(err_buf),
        ):
            try:
                rc = registry.main(args)
            except SystemExit as exc:
                rc = (
                    exc.code
                    if isinstance(exc.code, int)
                    else (0 if exc.code is None else 1)
                )
        return subprocess.CompletedProcess(
            args=args,
            returncode=rc,
            stdout=out_buf.getvalue(),
            stderr=err_buf.getvalue(),
        )

    def _write_sample_registry(self, fuchsia_dir: Path) -> Path:
        reg_path = fuchsia_dir / "tools" / "devshell" / "fx_host_tools.json"
        reg_path.parent.mkdir(parents=True, exist_ok=True)
        reg_path.write_text(
            json.dumps(
                {
                    "get-main-pb-path": {
                        "category": "Build",
                        "contrib": True,
                        "summary": "get the output path to the provided product bundle",
                        "target": "//tools/devshell/host_tools/get_main_pb_path:get-main-pb-path",
                    },
                    "get-src-dir": {
                        "category": "Build",
                        "summary": 'print the Fuchsia source directory (checkout root, aka "//")',
                        "target": "//tools/devshell/host_tools/get_src_dir:get-src-dir",
                    },
                    "registry": {
                        "category": "Source tree",
                        "summary": "list and manage registered fx host tools",
                        "script": "//tools/devshell/fx_cli/registry.py",
                    },
                    "shorthand-tool": "//scripts/developer:shorthand-tool",
                },
                indent=4,
                sort_keys=True,
            )
            + "\n"
        )
        return reg_path

    def test_list_all_tools_table_and_json_and_default_subcommand(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            fuchsia_dir = Path(td)
            self._write_sample_registry(fuchsia_dir)

            proc_default = self._run_registry([], fuchsia_dir=fuchsia_dir)
            self.assertEqual(
                proc_default.returncode, 0, msg=proc_default.stderr
            )
            self.assertIn("get-src-dir", proc_default.stdout)
            self.assertIn("registry", proc_default.stdout)
            self.assertIn("NAME", proc_default.stdout)

            proc_json = self._run_registry(
                ["list", "--json"], fuchsia_dir=fuchsia_dir
            )
            self.assertEqual(proc_json.returncode, 0, msg=proc_json.stderr)
            entries = json.loads(proc_json.stdout)
            self.assertEqual(len(entries), 4)
            names = [e["name"] for e in entries]
            self.assertEqual(
                names,
                [
                    "get-main-pb-path",
                    "get-src-dir",
                    "registry",
                    "shorthand-tool",
                ],
            )

    def test_list_formats_names_zsh_help_tsv(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            fuchsia_dir = Path(td)
            self._write_sample_registry(fuchsia_dir)

            # test --format=names
            proc_names = self._run_registry(
                ["list", "--format=names"], fuchsia_dir=fuchsia_dir
            )
            self.assertEqual(proc_names.returncode, 0, msg=proc_names.stderr)
            self.assertEqual(
                proc_names.stdout.splitlines(),
                [
                    "get-main-pb-path",
                    "get-src-dir",
                    "registry",
                    "shorthand-tool",
                ],
            )

            # test --format=names --hide-contrib
            proc_hide = self._run_registry(
                ["list", "--format=names", "--hide-contrib"],
                fuchsia_dir=fuchsia_dir,
            )
            self.assertEqual(proc_hide.returncode, 0, msg=proc_hide.stderr)
            self.assertEqual(
                proc_hide.stdout.splitlines(),
                ["get-src-dir", "registry", "shorthand-tool"],
            )

            # test --format=zsh
            proc_zsh = self._run_registry(
                ["list", "--format=zsh"], fuchsia_dir=fuchsia_dir
            )
            self.assertEqual(proc_zsh.returncode, 0, msg=proc_zsh.stderr)
            self.assertIn(
                "registry:list and manage registered fx host tools",
                proc_zsh.stdout,
            )

            # test --format=help-tsv
            proc_tsv = self._run_registry(
                ["list", "--format=help-tsv"], fuchsia_dir=fuchsia_dir
            )
            self.assertEqual(proc_tsv.returncode, 0, msg=proc_tsv.stderr)
            lines = proc_tsv.stdout.strip().splitlines()
            self.assertEqual(len(lines), 4)
            reg_fields = next(
                line for line in lines if line.startswith("registry\t")
            ).split("\t")
            self.assertEqual(reg_fields[0], "registry")
            self.assertEqual(reg_fields[1], "Source tree")
            self.assertEqual(reg_fields[5], "script")
            self.assertEqual(
                reg_fields[6], "//tools/devshell/fx_cli/registry.py"
            )
            with (
                mock.patch.dict(os.environ, {"FUCHSIA_DIR": str(fuchsia_dir)}),
                mock.patch.object(
                    registry, "load_merged_registry"
                ) as mock_merged,
                contextlib.redirect_stdout(io.StringIO()),
            ):
                rc = registry.main(["list", "--format=help-tsv"])
                self.assertEqual(rc, 0)
                mock_merged.assert_not_called()

    def test_multi_registry_discovery(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            fuchsia_dir = Path(td)
            self._write_sample_registry(fuchsia_dir)

            # Create vendor registry
            vendor_reg = (
                fuchsia_dir
                / "vendor"
                / "google"
                / "scripts"
                / "devshell"
                / "fx_host_tools.json"
            )
            vendor_reg.parent.mkdir(parents=True, exist_ok=True)
            vendor_reg.write_text(
                json.dumps(
                    {
                        "custom-vendor-tool": {
                            "category": "Build",
                            "summary": "vendor specific tool",
                            "script": "//vendor/google/scripts/devshell/custom.sh",
                        }
                    },
                    indent=4,
                    sort_keys=True,
                )
                + "\n"
            )

            # Discover all by default
            proc = self._run_registry(
                ["list", "--format=names"], fuchsia_dir=fuchsia_dir
            )
            self.assertEqual(proc.returncode, 0, msg=proc.stderr)
            names = proc.stdout.splitlines()
            self.assertIn("custom-vendor-tool", names)
            self.assertIn("get-src-dir", names)

            # Filter to vendor only
            proc_vendor = self._run_registry(
                ["list", "--vendor", "google", "--format=names"],
                fuchsia_dir=fuchsia_dir,
            )
            self.assertEqual(proc_vendor.returncode, 0, msg=proc_vendor.stderr)
            self.assertEqual(
                proc_vendor.stdout.splitlines(), ["custom-vendor-tool"]
            )

            # Filter to vendor only with help-tsv format
            proc_vendor_tsv = self._run_registry(
                ["list", "--vendor", "google", "--format=help-tsv"],
                fuchsia_dir=fuchsia_dir,
            )
            self.assertEqual(
                proc_vendor_tsv.returncode, 0, msg=proc_vendor_tsv.stderr
            )
            tsv_lines = proc_vendor_tsv.stdout.strip().splitlines()
            self.assertEqual(len(tsv_lines), 1)
            tsv_fields = tsv_lines[0].split("\t")
            self.assertEqual(tsv_fields[0], "vendor google custom-vendor-tool")
            self.assertEqual(tsv_fields[7], "custom-vendor-tool")

    def test_check_subcommand(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            fuchsia_dir = Path(td)
            # Create valid file targets
            pkg_dir = (
                fuchsia_dir
                / "tools"
                / "devshell"
                / "host_tools"
                / "get_src_dir"
            )
            pkg_dir.mkdir(parents=True, exist_ok=True)
            (pkg_dir / "BUILD.bazel").write_text("# BUILD\n", encoding="utf-8")

            cli_dir = fuchsia_dir / "tools" / "devshell" / "fx_cli"
            cli_dir.mkdir(parents=True, exist_ok=True)
            (cli_dir / "registry.py").write_text(
                "# registry\n", encoding="utf-8"
            )

            reg_path = fuchsia_dir / "tools" / "devshell" / "fx_host_tools.json"
            reg_path.parent.mkdir(parents=True, exist_ok=True)
            reg_path.write_text(
                json.dumps(
                    {
                        "get-src-dir": {
                            "category": "Build",
                            "summary": "print src dir",
                            "target": "//tools/devshell/host_tools/get_src_dir:get-src-dir",
                        },
                        "registry": {
                            "category": "Source tree",
                            "summary": "manage registry",
                            "script": "//tools/devshell/fx_cli/registry.py",
                        },
                    },
                    indent=4,
                    sort_keys=True,
                )
                + "\n"
            )

            proc_check = self._run_registry(["check"], fuchsia_dir=fuchsia_dir)
            self.assertEqual(proc_check.returncode, 0, msg=proc_check.stderr)

            # Test failure on unsorted keys
            reg_path.write_text(
                '{"registry": {"script": "//tools/devshell/fx_cli/registry.py"}, "get-src-dir": {"target": "//tools/devshell/host_tools/get_src_dir:get-src-dir"}}\n'
            )
            proc_bad_sort = self._run_registry(
                ["check"], fuchsia_dir=fuchsia_dir
            )
            self.assertNotEqual(proc_bad_sort.returncode, 0)
            self.assertIn("must be sorted", proc_bad_sort.stderr)

            # Test failure on missing BUILD.bazel
            reg_path.write_text(
                json.dumps(
                    {
                        "missing-pkg": {
                            "category": "Build",
                            "summary": "missing",
                            "target": "//tools/missing:missing",
                        }
                    },
                    indent=4,
                    sort_keys=True,
                )
                + "\n"
            )
            proc_missing = self._run_registry(
                ["check"], fuchsia_dir=fuchsia_dir
            )
            self.assertNotEqual(proc_missing.returncode, 0)
            self.assertIn(
                "references missing Bazel package", proc_missing.stderr
            )

    def test_add_and_remove(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            fuchsia_dir = Path(td)
            reg_path = fuchsia_dir / "tools" / "devshell" / "fx_host_tools.json"
            reg_path.parent.mkdir(parents=True, exist_ok=True)
            reg_path.write_text("{}\n", encoding="utf-8")

            # Add a tool
            proc_add = self._run_registry(
                [
                    "add",
                    "my-tool",
                    "--script",
                    "//tools/devshell/my_tool.sh",
                    "-c",
                    "Build",
                    "-s",
                    "My cool tool",
                ],
                fuchsia_dir=fuchsia_dir,
            )
            self.assertEqual(proc_add.returncode, 0, msg=proc_add.stderr)

            # Verify it was added
            raw = json.loads(reg_path.read_text(encoding="utf-8"))
            self.assertIn("my-tool", raw)
            self.assertEqual(
                raw["my-tool"]["script"], "//tools/devshell/my_tool.sh"
            )
            self.assertEqual(raw["my-tool"]["category"], "Build")
            self.assertEqual(raw["my-tool"]["summary"], "My cool tool")

            # Remove the tool
            proc_rm = self._run_registry(
                ["remove", "my-tool"], fuchsia_dir=fuchsia_dir
            )
            self.assertEqual(proc_rm.returncode, 0, msg=proc_rm.stderr)

            raw = json.loads(reg_path.read_text(encoding="utf-8"))
            self.assertNotIn("my-tool", raw)

            # Reject invalid category on add
            proc_bad_cat = self._run_registry(
                [
                    "add",
                    "bad-cat",
                    "--script",
                    "//tools/devshell/my_tool.sh",
                    "-c",
                    "NotACategory",
                    "-s",
                    "Valid summary",
                ],
                fuchsia_dir=fuchsia_dir,
            )
            self.assertNotEqual(proc_bad_cat.returncode, 0)
            self.assertIn("category", proc_bad_cat.stderr)

            # Reject tab or newline in summary on add
            for bad_summary in ("has\ttab", "has\nnewline", "   "):
                proc_bad_sum = self._run_registry(
                    [
                        "add",
                        "bad-sum",
                        "--script",
                        "//tools/devshell/my_tool.sh",
                        "-c",
                        "Build",
                        "-s",
                        bad_summary,
                    ],
                    fuchsia_dir=fuchsia_dir,
                )
                self.assertNotEqual(proc_bad_sum.returncode, 0)
                self.assertIn("summary", proc_bad_sum.stderr)


if __name__ == "__main__":
    unittest.main()
