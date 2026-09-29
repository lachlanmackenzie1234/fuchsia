#!/usr/bin/env fuchsia-vendored-python
# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

_SCRIPT_DIR = Path(__file__).parent
sys.path.insert(0, str(_SCRIPT_DIR))

import bazel_tests_utils
from build_utils import BazelPaths, MockCommandRunner


class BazelTestsUtilsTest(unittest.TestCase):
    def setUp(self) -> None:
        self._td = tempfile.TemporaryDirectory()
        self.fuchsia_dir = Path(self._td.name) / "fuchsia"
        self.fuchsia_dir.mkdir()
        (self.fuchsia_dir / ".jiri_manifest").write_text("")

        self.build_dir = self.fuchsia_dir / "out" / "build_dir"
        self.build_dir.mkdir(parents=True)

        BazelPaths.write_topdir_config_for_test(
            self.fuchsia_dir, "gen/build/bazel"
        )
        self.bazel_paths = BazelPaths(self.fuchsia_dir, self.build_dir)

        # Create the directories that BazelPaths properties expect
        self.bazel_paths.launcher.parent.mkdir(parents=True, exist_ok=True)
        self.bazel_paths.launcher.write_text("#!/bin/bash\nexit 0")
        self.bazel_paths.execroot.mkdir(parents=True, exist_ok=True)
        (
            self.bazel_paths.ninja_build_dir / "bazel_host_test_suites.txt"
        ).write_text("//fake/test1\n//fake/test2")
        (
            self.bazel_paths.ninja_build_dir / "bazel_target_test_suites.txt"
        ).write_text("")
        (
            self.bazel_paths.ninja_build_dir
            / "target_tests.gn_targets_manifest.json"
        ).write_text("[]")
        (
            self.bazel_paths.ninja_build_dir / "bazel_target_infos.json"
        ).write_text(
            json.dumps(
                [
                    {
                        "bazel_target": "//build/bazel/target_tests:target_tests_stamp",
                        "gn_targets_manifest": "target_tests.gn_targets_manifest.json",
                    }
                ]
            )
        )

    def tearDown(self) -> None:
        self._td.cleanup()

    def test_generate_tests_json(self) -> None:
        mock_runner = MockCommandRunner()

        # Create some fake test info that matches what cquery would return
        test_info = {
            "label": "//src/my_test:my_test",
            "launcher_execroot_path": "bin/my_test",
            "runtime_deps_json_execroot_path": "bin/my_test.runtime_deps.json",
            "os": "linux",
            "cpu": "x64",
            "list_cases_argument": "",
        }

        mock_runner.push_result(stdout=json.dumps(test_info))

        tests_json, _ = bazel_tests_utils.generate_tests_json(
            self.bazel_paths, command_runner=mock_runner
        )

        self.assertEqual(len(tests_json), 1)
        entry = tests_json[0]
        self.assertEqual(entry["test"]["name"], "//src/my_test:my_test")
        self.assertEqual(entry["test"]["label"], "//src/my_test:my_test")
        self.assertEqual(entry["test"]["source_label"], "//src/my_test:my_test")

        # Verify path conversion
        # bazel_paths.execroot is fuchsia_dir/out/build_dir/gen/bazel/output_base/execroot/_main
        # path is relative to ninja_build_dir (fuchsia_dir/out/build_dir)
        expected_path = os.path.relpath(
            self.bazel_paths.execroot / "bin/my_test",
            self.bazel_paths.ninja_build_dir,
        )
        self.assertEqual(entry["test"]["path"], expected_path)

        expected_deps_path = os.path.relpath(
            self.bazel_paths.execroot / "bin/my_test.runtime_deps.json",
            self.bazel_paths.ninja_build_dir,
        )
        self.assertEqual(entry["test"]["runtime_deps"], expected_deps_path)

    def test_generate_tests_json_multiple_entries(self) -> None:
        mock_runner = MockCommandRunner()
        test1 = {
            "name": "test1",
            "label": "//t1",
            "source_label": "//t1",
            "launcher_execroot_path": "p1",
            "runtime_deps_json_execroot_path": "d1",
            "os": "linux",
            "cpu": "x64",
            "list_cases_argument": "",
        }
        test2 = {
            "name": "test2",
            "label": "//t2",
            "source_label": "//t2",
            "launcher_execroot_path": "p2",
            "runtime_deps_json_execroot_path": "d2",
            "os": "linux",
            "cpu": "x64",
            "list_cases_argument": "list_mock_unittests",
        }

        mock_runner.push_result(
            stdout=json.dumps(test1) + "\n" + json.dumps(test2)
        )

        tests_json, _ = bazel_tests_utils.generate_tests_json(
            self.bazel_paths, command_runner=mock_runner
        )

        self.assertEqual(len(tests_json), 2)
        self.assertEqual(tests_json[0]["test"]["name"], "//t1")
        self.assertEqual(tests_json[1]["test"]["name"], "//t2")

        execroot_path = "gen/build/bazel/output_base/execroot/_main"
        self.assertEqual(
            tests_json[0],
            {
                "environments": [
                    {
                        "dimensions": {
                            "os": "Linux",
                            "cpu": "x64",
                        }
                    }
                ],
                "expects_ssh": False,
                "test": {
                    "name": f"//t1",
                    "label": "//t1",
                    "source_label": "//t1",
                    "path": f"{execroot_path}/p1",
                    "runtime_deps": f"{execroot_path}/d1",
                    "os": "linux",
                    "cpu": "x64",
                },
            },
        )
        self.assertEqual(
            tests_json[1],
            {
                "environments": [
                    {
                        "dimensions": {
                            "os": "Linux",
                            "cpu": "x64",
                        }
                    }
                ],
                "expects_ssh": False,
                "test": {
                    "name": f"//t2",
                    "label": "//t2",
                    "source_label": "//t2",
                    "path": f"{execroot_path}/p2",
                    "runtime_deps": f"{execroot_path}/d2",
                    "os": "linux",
                    "cpu": "x64",
                    "list_cases_argument": "list_mock_unittests",
                },
            },
        )

    def test_generate_tests_json_failure(self) -> None:
        mock_runner = MockCommandRunner()
        mock_runner.push_result(returncode=1, stderr="Bazel error")

        with self.assertRaisesRegex(
            RuntimeError, "Failed to run bazel query: Bazel error"
        ):
            bazel_tests_utils.generate_tests_json(
                self.bazel_paths, command_runner=mock_runner
            )

    def test_generate_tests_json_missing_fuchsia_host_test_info(self) -> None:
        mock_runner = MockCommandRunner()
        missing_info = {
            "error": "missing_fuchsia_host_test_info",
            "label": "//tools/check-licenses:v2_config_test",
        }
        mock_runner.push_result(stdout=json.dumps(missing_info))

        with self.assertRaisesRegex(
            RuntimeError,
            r"Target '//tools/check-licenses:v2_config_test' included in the bazel_host_test_suites GN group is a test target but does not provide FuchsiaHostTestInfo",
        ):
            bazel_tests_utils.generate_tests_json(
                self.bazel_paths, command_runner=mock_runner
            )

    def test_generate_tests_json_multiple_missing_fuchsia_host_test_info(
        self,
    ) -> None:
        mock_runner = MockCommandRunner()
        missing_info1 = {
            "error": "missing_fuchsia_host_test_info",
            "label": "//tools/check-licenses:v2_config_test",
        }
        missing_info2 = {
            "error": "missing_fuchsia_host_test_info",
            "label": "//tools/whereiscl:whereiscl_test",
        }
        mock_runner.push_result(
            stdout=json.dumps(missing_info1) + "\n" + json.dumps(missing_info2)
        )

        with self.assertRaisesRegex(
            RuntimeError,
            r"The following targets included in the bazel_host_test_suites GN group are test targets but do not provide FuchsiaHostTestInfo:\n  - //tools/check-licenses:v2_config_test\n  - //tools/whereiscl:whereiscl_test",
        ):
            bazel_tests_utils.generate_tests_json(
                self.bazel_paths, command_runner=mock_runner
            )

    def test_generate_tests_json_with_debug_symbols(self) -> None:
        mock_runner = MockCommandRunner()
        test_info = {
            "label": "//src/my_test:my_test",
            "launcher_execroot_path": "bin/my_test",
            "runtime_deps_json_execroot_path": "bin/my_test.runtime_deps.json",
            "unstripped_binary_execroot_path": "bin/my_test_unstripped",
            "os": "linux",
            "cpu": "x64",
            "list_cases_argument": "",
        }
        mock_runner.push_result(stdout=json.dumps(test_info))

        bazel_tests_utils.generate_tests_json(
            self.bazel_paths, command_runner=mock_runner
        )

        manifest_file = (
            self.bazel_paths.ninja_build_dir
            / "bazel_host_tests.debug_symbols.json"
        )
        self.assertTrue(manifest_file.exists())
        execroot_path = "gen/build/bazel/output_base/execroot/_main"
        self.assertEqual(
            json.loads(manifest_file.read_text()),
            [
                {
                    "cpu": "x64",
                    "debug": f"{execroot_path}/bin/my_test_unstripped",
                    "label": "//src/my_test:my_test",
                    "os": "linux",
                }
            ],
        )

    def _setUpDeviceTests(self, suites: list[str]) -> None:
        """Make only the device cquery run, over the given test suite labels."""
        (
            self.bazel_paths.ninja_build_dir / "bazel_host_test_suites.txt"
        ).write_text("")
        (
            self.bazel_paths.ninja_build_dir / "bazel_target_test_suites.txt"
        ).write_text("\n".join(suites))

    def test_generate_device_tests_json(self) -> None:
        self._setUpDeviceTests(["//fake/device_tests"])

        mock_runner = MockCommandRunner()
        test_info = {
            "label": "@@//src/my_test:my_test",
            "package_manifest_execroot_path": "bazel-out/my_test/package_manifest.json",
            "os": "fuchsia",
            "cpu": "x64",
            "test_components": [
                {
                    "component_name": "my_test",
                    "package_url": "fuchsia-pkg://fuchsia.com/my-test-package#meta/my_test.cm",
                },
                {
                    "component_name": "my_other_test",
                    "package_url": "fuchsia-pkg://fuchsia.com/my-test-package#meta/my_other_test.cm",
                },
            ],
        }
        mock_runner.push_result(stdout=json.dumps(test_info))

        tests_json, _ = bazel_tests_utils.generate_tests_json(
            self.bazel_paths, command_runner=mock_runner
        )

        execroot_path = "gen/build/bazel/output_base/execroot/_main"
        package_manifest = (
            f"{execroot_path}/bazel-out/my_test/package_manifest.json"
        )
        self.assertEqual(
            tests_json,
            [
                {
                    "environments": [],
                    "expects_ssh": True,
                    "test": {
                        "build_rule": "fx_test",
                        "cpu": "x64",
                        "label": "@@//src/my_test:my_test",
                        "source_label": "//src/my_test:my_test",
                        "name": f"fuchsia-pkg://fuchsia.com/my-test-package#meta/{component}.cm",
                        "os": "fuchsia",
                        "package_url": f"fuchsia-pkg://fuchsia.com/my-test-package#meta/{component}.cm",
                        "package_manifests": [package_manifest],
                        "log_settings": {"max_severity": "WARN"},
                    },
                }
                for component in ("my_test", "my_other_test")
            ],
        )

        packages_list = (
            self.bazel_paths.ninja_build_dir / "bazel_test_packages.list"
        )
        self.assertEqual(
            json.loads(packages_list.read_text()),
            {"content": {"manifests": [package_manifest]}, "version": "1"},
        )

        query_command = mock_runner.commands[-1]
        self.assertIn("cquery", query_command)
        self.assertIn("--config=fuchsia_platform", query_command)
        self.assertIn("FuchsiaTestInfo.cquery", query_command)

    def test_generate_device_tests_json_max_log_severity(self) -> None:
        self._setUpDeviceTests(["//fake/device_tests"])

        mock_runner = MockCommandRunner()
        mock_runner.push_result(
            stdout=json.dumps(
                {
                    "label": "@@//src/my_test:my_test",
                    "package_manifest_execroot_path": "bazel-out/my_test/package_manifest.json",
                    "os": "fuchsia",
                    "cpu": "riscv64",
                    "max_log_severity": "ERROR",
                    "test_components": [
                        {
                            "component_name": "my_test",
                            "package_url": "fuchsia-pkg://fuchsia.com/my-test-package#meta/my_test.cm",
                        },
                    ],
                }
            )
        )

        tests_json, _ = bazel_tests_utils.generate_tests_json(
            self.bazel_paths, command_runner=mock_runner
        )

        self.assertEqual(len(tests_json), 1)
        self.assertEqual(
            tests_json[0]["test"]["log_settings"],
            {"max_severity": "ERROR"},
        )
        # Environments are resolved later by build_tests_json.py, for any CPU.
        self.assertEqual(tests_json[0]["environments"], [])
        self.assertEqual(tests_json[0]["test"]["cpu"], "riscv64")

    def test_generate_device_tests_json_build_file_inputs(self) -> None:
        self._setUpDeviceTests(["//fake/device_tests:suite"])
        suite_build = self.fuchsia_dir / "fake" / "device_tests" / "BUILD.bazel"
        suite_build.parent.mkdir(parents=True)
        suite_build.write_text("")
        test_build = self.fuchsia_dir / "src" / "my_test" / "BUILD"
        test_build.parent.mkdir(parents=True)
        test_build.write_text("")

        mock_runner = MockCommandRunner()
        mock_runner.push_result(
            stdout=json.dumps(
                {
                    "label": "@@//src/my_test:my_test",
                    "package_manifest_execroot_path": "bazel-out/my_test/package_manifest.json",
                    "os": "fuchsia",
                    "cpu": "x64",
                    "test_components": [
                        {
                            "component_name": "my_test",
                            "package_url": "fuchsia-pkg://fuchsia.com/my-test-package#meta/my_test.cm",
                        },
                    ],
                }
            )
        )

        _, inputs = bazel_tests_utils.generate_tests_json(
            self.bazel_paths, command_runner=mock_runner
        )

        self.assertIn(suite_build, inputs)
        self.assertIn(test_build, inputs)

    def test_generate_device_tests_json_missing_test_info(self) -> None:
        self._setUpDeviceTests(["//fake/device_tests"])

        mock_runner = MockCommandRunner()
        mock_runner.push_result(
            stdout=json.dumps(
                {
                    "error": "missing_fuchsia_test_info",
                    "label": "@@//src/my_test:my_test",
                }
            )
        )

        with self.assertRaisesRegex(
            RuntimeError,
            r"do not provide FuchsiaTestInfo:\n  - //src/my_test:my_test",
        ):
            bazel_tests_utils.generate_tests_json(
                self.bazel_paths, command_runner=mock_runner
            )

    def test_generate_device_tests_json_unexpected_error(self) -> None:
        self._setUpDeviceTests(["//fake/device_tests"])

        mock_runner = MockCommandRunner()
        mock_runner.push_result(
            stdout=json.dumps(
                {
                    "error": "something_else",
                    "label": "@@//src/my_test:my_test",
                }
            )
        )

        with self.assertRaisesRegex(
            RuntimeError, r"Unexpected error in cquery output"
        ):
            bazel_tests_utils.generate_tests_json(
                self.bazel_paths, command_runner=mock_runner
            )

    def test_bazel_test_packages_list_written_when_no_device_tests(
        self,
    ) -> None:
        mock_runner = MockCommandRunner()
        mock_runner.push_result(stdout="")

        bazel_tests_utils.generate_tests_json(
            self.bazel_paths, command_runner=mock_runner
        )

        packages_list = (
            self.bazel_paths.ninja_build_dir / "bazel_test_packages.list"
        )
        self.assertEqual(
            json.loads(packages_list.read_text()),
            {"content": {"manifests": []}, "version": "1"},
        )
        # `build/bazel/tests_json.gn_targets` is also populated so that direct
        # `fx build --fuchsia_platform <label>` invocations can use it even when
        # no suites are in `bazel_target_test_suites.txt`.
        self.assertTrue(
            (
                self.bazel_paths.ninja_build_dir
                / "build/bazel/tests_json.gn_targets/BUILD.bazel"
            ).is_file()
        )

    def test_missing_target_tests_stamp_entry_raises(self) -> None:
        self._setUpDeviceTests([])
        (
            self.bazel_paths.ninja_build_dir / "bazel_target_infos.json"
        ).write_text("[]")

        with self.assertRaisesRegex(
            RuntimeError,
            r"No entry for //build/bazel/target_tests:target_tests_stamp found in",
        ):
            bazel_tests_utils.generate_tests_json(
                self.bazel_paths, command_runner=MockCommandRunner()
            )

    def test_write_bazel_test_packages_list_only_writes_if_changed(
        self,
    ) -> None:
        packages_list = (
            self.bazel_paths.ninja_build_dir / "bazel_test_packages.list"
        )
        bazel_tests_utils.write_bazel_test_packages_list(
            self.bazel_paths.ninja_build_dir, ["obj/foo/package_manifest.json"]
        )
        os.utime(packages_list, (1000, 1000))

        bazel_tests_utils.write_bazel_test_packages_list(
            self.bazel_paths.ninja_build_dir, ["obj/foo/package_manifest.json"]
        )
        self.assertEqual(packages_list.stat().st_mtime, 1000)

        bazel_tests_utils.write_bazel_test_packages_list(
            self.bazel_paths.ninja_build_dir, ["obj/bar/package_manifest.json"]
        )
        self.assertNotEqual(packages_list.stat().st_mtime, 1000)


if __name__ == "__main__":
    unittest.main()
