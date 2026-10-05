#!/usr/bin/env fuchsia-vendored-python
# Copyright 2025 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

import json
import os
import sys
import tempfile
import typing as T
import unittest
from pathlib import Path

sys.path.insert(0, os.path.dirname(__file__))
import bazel_build_events
import stdio_redirection
from bazel_action_utils import (
    AspectManifestOutputs,
    BazelStderrDebugLineFilter,
    BazelStderrDebugLineRecorder,
    find_prefix_in_input,
    parse_build_event_manifests,
)


class FindPrefixInInputTest(unittest.TestCase):
    def test_find_prefix_in_input(self) -> None:
        TEST_CASES = [
            ("foo", "-------", (0, 7)),  # no match
            ("foo", "foo----", (2, 0)),  # full matches
            ("foo", "--foo--", (2, 2)),
            ("foo", "----foo", (2, 4)),
            ("foo", "-----fo", (1, 5)),  # partial matches
            ("foo", "------f", (1, 6)),
        ]
        for prefix, input, expected in TEST_CASES:
            self.assertEqual(
                find_prefix_in_input(prefix, input),
                expected,
                msg=f"For prefix={prefix} and input={input}",
            )


class BazelStderrDebugLineFilterTest(unittest.TestCase):
    def setUp(self) -> None:
        self.output = stdio_redirection.BytesOutputSink()

    def test_no_filtering(self) -> None:
        filter_sink = BazelStderrDebugLineFilter(self.output)
        self.assertTrue(filter_sink.write(b"foooo\nsomethingDEBUG: bar"))
        self.assertEqual(self.output.data, b"foooo\nsomething")
        self.assertTrue(filter_sink.write(b"\nfinish"))
        self.assertEqual(
            self.output.data, b"foooo\nsomethingDEBUG: bar\nfinish"
        )

    def test_no_filtering_colored(self) -> None:
        filter_sink = BazelStderrDebugLineFilter(self.output)
        self.assertTrue(
            filter_sink.write(b"foooo\nsomething\x1b[33mDEBUG: \x1b[0mbar")
        )
        self.assertEqual(self.output.data, b"foooo\nsomething")
        self.assertTrue(filter_sink.write(b"\nfinish"))
        self.assertEqual(
            self.output.data,
            b"foooo\nsomething\x1b[33mDEBUG: \x1b[0mbar\nfinish",
        )

    def test_no_filtering_bad_color(self) -> None:
        filter_sink = BazelStderrDebugLineFilter(self.output)
        self.assertTrue(
            filter_sink.write(b"foooo\nsomething\x1b[31mDEBUG: \x1b[0mbar")
        )
        self.assertEqual(self.output.data, b"foooo\nsomething\x1b[31m")
        self.assertTrue(filter_sink.write(b"\nfinish"))
        self.assertEqual(
            self.output.data,
            b"foooo\nsomething\x1b[31mDEBUG: \x1b[0mbar\nfinish",
        )

    def test_partial_writes(self) -> None:
        filter_sink = BazelStderrDebugLineFilter(self.output)
        self.assertTrue(filter_sink.write(b"foooo\nsomethingDEB"))
        self.assertEqual(self.output.data, b"foooo\nsomething")
        self.assertTrue(filter_sink.write(b"UG: bar"))
        self.assertEqual(self.output.data, b"foooo\nsomething")
        self.assertTrue(filter_sink.write(b"\nfinish"))
        self.assertEqual(
            self.output.data, b"foooo\nsomethingDEBUG: bar\nfinish"
        )

    def test_with_filtering_all(self) -> None:
        filter_sink = BazelStderrDebugLineFilter(self.output, lambda x: True)
        self.assertTrue(
            filter_sink.write(
                b"foooo\nsomething\nDEBUG: bar\nsomething else\nDEBUG: zoo\n"
            )
        )
        self.assertEqual(
            self.output.data, b"foooo\nsomething\nsomething else\n"
        )

    def test_with_filtering_some(self) -> None:
        filter_sink = BazelStderrDebugLineFilter(
            self.output, lambda x: b"SKIP" in x
        )
        self.assertTrue(
            filter_sink.write(
                b"foooo\nsomething\nDEBUG: KEEP ME\nsomething else\nDEBUG: SKIP ME\n"
            )
        )
        self.assertEqual(
            self.output.data,
            b"foooo\nsomething\nDEBUG: KEEP ME\nsomething else\n",
        )


class BazelStderrDebugLineRecorderTest(unittest.TestCase):
    def setUp(self) -> None:
        self.output = stdio_redirection.BytesOutputSink()

    def test_recording(self) -> None:
        prefix_map = {
            "first": b"PREFIX1=",
            "second": b"PREFIX2=",
        }
        recorder = BazelStderrDebugLineRecorder(self.output, prefix_map)

        self.assertTrue(recorder.write(b"line 1\n"))
        self.assertTrue(recorder.write(b"DEBUG: PREFIX1=value 1\n"))
        self.assertTrue(recorder.write(b"line 2\n"))
        self.assertTrue(recorder.write(b"DEBUG: PREFIX2=value 2\n"))
        self.assertTrue(recorder.write(b"DEBUG: PREFIX1=value 3\n"))
        self.assertTrue(recorder.write(b"line 3\n"))

        # Verify the filtered output stream contains the non-debug lines
        self.assertEqual(
            self.output.data,
            b"line 1\nline 2\nline 3\n",
        )

        # Verify recorded values
        self.assertEqual(
            recorder.get_recorded_values("first"), ["value 1", "value 3"]
        )
        self.assertEqual(recorder.get_recorded_values("second"), ["value 2"])

        self.assertDictEqual(
            recorder.get_all_recorded_values(),
            {
                "first": ["value 1", "value 3"],
                "second": ["value 2"],
            },
        )

        # Verify invalid names throw AssertionError
        with self.assertRaises(AssertionError):
            recorder.get_recorded_values("third")

    def test_recording_with_partial_colored_prefix(self) -> None:
        prefix_map = {
            "first": b"PREFIX1=",
            "second": b"PREFIX2=",
        }
        recorder = BazelStderrDebugLineRecorder(self.output, prefix_map)

        self.assertTrue(recorder.write(b"line 1\nDEBUG"))
        self.assertTrue(recorder.write(b": PREFIX1=value 1\n"))

        # The following lines writes a partial colored prefix that
        # includes a full regular prefix (which should be ignored).
        # This used to raise an assertion, see https://fxbug.dev/515325221
        self.assertTrue(recorder.write(b"line 2\n\x1b[33mDEBUG: "))
        self.assertTrue(recorder.write(b"\x1b[0mPREFIX2=value 2\n"))
        self.assertTrue(recorder.write(b"DEBUG: PREFIX1=value 3\n"))
        self.assertTrue(recorder.write(b"line 3\n"))

        # Verify the filtered output stream contains the non-debug lines
        self.assertEqual(
            self.output.data,
            b"line 1\nline 2\nline 3\n",
        )

        # Verify recorded values
        self.assertEqual(
            recorder.get_recorded_values("first"), ["value 1", "value 3"]
        )
        self.assertEqual(recorder.get_recorded_values("second"), ["value 2"])

        self.assertDictEqual(
            recorder.get_all_recorded_values(),
            {
                "first": ["value 1", "value 3"],
                "second": ["value 2"],
            },
        )


class BazelTargetInfosMapTest(unittest.TestCase):
    def test_update_rust_project(self) -> None:
        from bazel_action_utils import BazelTargetInfosMap

        sample_content: list[dict[str, T.Any]] = [
            {
                "type": "file",
                "bazel_target": "//src:foo",
                "bazel_platform_label": "//build/bazel/platforms:host",
                "bazel_platform_config": "host",
                "ninja_depfile": "obj/src/foo.d",
                "gn_targets_manifest": "gen/gn_targets.manifest",
                "stamp_path": "obj/src/foo.stamp",
                "bazel_file": "foo",
                "ninja_file": "foo",
                "update_rust_project": True,
            },
            {
                "type": "file",
                "bazel_target": "//src:bar",
                "bazel_platform_label": "//build/bazel/platforms:host",
                "bazel_platform_config": "host",
                "ninja_depfile": "obj/src/bar.d",
                "gn_targets_manifest": "gen/gn_targets.manifest",
                "stamp_path": "obj/src/bar.stamp",
                "bazel_file": "bar",
                "ninja_file": "bar",
                "update_rust_project": False,
            },
        ]
        target_map = BazelTargetInfosMap(sample_content)
        foo_info = target_map.get_info(
            "//src:foo", "//build/bazel/platforms:host"
        )
        self.assertIsNotNone(foo_info)
        assert foo_info is not None
        self.assertTrue(foo_info.update_rust_project)

        bar_info = target_map.get_info(
            "//src:bar", "//build/bazel/platforms:host"
        )
        self.assertIsNotNone(bar_info)
        assert bar_info is not None
        self.assertFalse(bar_info.update_rust_project)

    def test_copy_debug_symbols(self) -> None:
        from bazel_action_utils import BazelTargetInfosMap

        def entry(name: str, **extra: T.Any) -> dict[str, T.Any]:
            return {
                "type": "file",
                "bazel_target": f"//src:{name}",
                "bazel_platform_label": "//build/bazel/platforms:host",
                "bazel_platform_config": "host",
                "ninja_depfile": f"obj/src/{name}.d",
                "gn_targets_manifest": "gen/gn_targets.manifest",
                "stamp_path": f"obj/src/{name}.stamp",
                "bazel_file": name,
                "ninja_file": name,
                "update_rust_project": False,
                **extra,
            }

        target_map = BazelTargetInfosMap(
            [
                entry("foo", copy_debug_symbols=True),
                entry("bar", copy_debug_symbols=False),
                entry("baz"),
            ]
        )
        for name, expected in [("foo", True), ("bar", False), ("baz", False)]:
            info = target_map.get_info(
                f"//src:{name}", "//build/bazel/platforms:host"
            )
            assert info is not None
            self.assertEqual(info.copy_debug_symbols, expected, name)

    def test_extra_bazel_targets_file(self) -> None:
        from bazel_action_utils import BazelTargetInfosMap

        def entry(name: str, **extra: T.Any) -> dict[str, T.Any]:
            return {
                "type": "file",
                "bazel_target": f"//src:{name}",
                "bazel_platform_label": "//build/bazel/platforms:host",
                "bazel_platform_config": "host",
                "ninja_depfile": f"obj/src/{name}.d",
                "gn_targets_manifest": "gen/gn_targets.manifest",
                "stamp_path": f"obj/src/{name}.stamp",
                "bazel_file": name,
                "ninja_file": name,
                "update_rust_project": False,
                **extra,
            }

        target_map = BazelTargetInfosMap(
            [
                entry("foo", extra_bazel_targets_file="extra_targets.txt"),
                entry("bar"),
            ]
        )

        foo_info = target_map.get_info(
            "//src:foo", "//build/bazel/platforms:host"
        )
        assert foo_info is not None
        self.assertEqual(foo_info.extra_bazel_targets_file, "extra_targets.txt")

        # The field is optional, and most actions omit it entirely.
        bar_info = target_map.get_info(
            "//src:bar", "//build/bazel/platforms:host"
        )
        assert bar_info is not None
        self.assertIsNone(bar_info.extra_bazel_targets_file)

        self.assertEqual(list(target_map.all_infos()), [foo_info, bar_info])


class AspectManifestOutputsTest(unittest.TestCase):
    def test_to_from_dict_and_file(self) -> None:
        outputs = AspectManifestOutputs(
            source_files_manifest_paths=["fake-out/bin/fake_sources.json"],
            debug_symbol_manifest_paths=[
                "fake-out/bin/fake_symbols.debug_symbols.json"
            ],
            rust_analyzer_manifest_paths=[
                "fake-out/bin/fake_rust.fuchsia_rust_analyzer_manifest.json"
            ],
            genquery_output_files=[
                "//buildfiles_genquery:fake_q,fake-out/bin/fake_q.txt"
            ],
        )
        d = outputs.to_dict()
        reconstructed = AspectManifestOutputs.from_dict(d)
        self.assertEqual(outputs, reconstructed)

        with tempfile.TemporaryDirectory() as tmpdir:
            file_path = Path(tmpdir) / "all_manifests.json"
            outputs.save_to_file(file_path)
            loaded = AspectManifestOutputs.load_from_file(file_path)
            self.assertEqual(outputs, loaded)

    def test_from_build_event_stream(self) -> None:
        events = [
            {
                "id": {"namedSet": {"id": "set-src"}},
                "namedSetOfFiles": {
                    "files": [
                        {
                            "name": "fake/pkg/fake_target.fuchsia_source_files.json",
                            "pathPrefix": ["fake-out", "bin"],
                        }
                    ]
                },
            },
            {
                "id": {"namedSet": {"id": "set-dbg"}},
                "namedSetOfFiles": {
                    "files": [
                        {
                            "name": "fake/pkg/fake_bin.debug_symbols.json",
                            "pathPrefix": ["fake-out", "bin"],
                        },
                        {
                            "name": "fake/pkg/fake_bin",
                            "pathPrefix": ["fake-out", "bin"],
                        },
                    ]
                },
            },
            {
                "id": {"namedSet": {"id": "set-rust"}},
                "namedSetOfFiles": {
                    "files": [
                        {
                            "name": "fake/pkg/fake_rust.fuchsia_rust_analyzer_manifest.json",
                            "pathPrefix": ["fake-out", "bin"],
                        }
                    ]
                },
            },
            {
                "id": {"namedSet": {"id": "set-gq"}},
                "namedSetOfFiles": {
                    "files": [
                        {
                            "name": "buildfiles_genquery/fake_target.buildfiles.txt",
                            "pathPrefix": ["fake-out", "bin"],
                        }
                    ]
                },
            },
            {
                "id": {"targetCompleted": {"label": "//fake/pkg:fake_target"}},
                "completed": {
                    "outputGroup": [
                        {
                            "name": "fuchsia_sources_manifest",
                            "fileSets": [{"id": "set-src"}],
                        }
                    ]
                },
            },
            {
                "id": {"targetCompleted": {"label": "//fake/pkg:fake_bin"}},
                "completed": {
                    "outputGroup": [
                        {
                            "name": "debug_symbol_manifest",
                            "fileSets": [{"id": "set-dbg"}],
                        }
                    ]
                },
            },
            {
                "id": {"targetCompleted": {"label": "//fake/pkg:fake_rust"}},
                "completed": {
                    "outputGroup": [
                        {
                            "name": "fuchsia_rust_analyzer_manifest",
                            "fileSets": [{"id": "set-rust"}],
                        }
                    ]
                },
            },
            {
                "id": {
                    "targetCompleted": {
                        "label": "//buildfiles_genquery:fake_target.buildfiles.txt"
                    }
                },
                "completed": {
                    "outputGroup": [
                        {
                            "name": "default",
                            "fileSets": [{"id": "set-gq"}],
                        }
                    ]
                },
            },
        ]
        stream = bazel_build_events.BuildEventStream.from_events(events)
        outputs = AspectManifestOutputs.from_build_event_stream(stream)

        expected_src = os.path.join(
            "fake-out", "bin", "fake/pkg/fake_target.fuchsia_source_files.json"
        )
        expected_dbg = os.path.join(
            "fake-out", "bin", "fake/pkg/fake_bin.debug_symbols.json"
        )
        expected_rust = os.path.join(
            "fake-out",
            "bin",
            "fake/pkg/fake_rust.fuchsia_rust_analyzer_manifest.json",
        )
        expected_gq = os.path.join(
            "fake-out", "bin", "buildfiles_genquery/fake_target.buildfiles.txt"
        )

        self.assertEqual(outputs.source_files_manifest_paths, [expected_src])
        self.assertEqual(outputs.debug_symbol_manifest_paths, [expected_dbg])
        self.assertEqual(outputs.rust_analyzer_manifest_paths, [expected_rust])
        self.assertEqual(
            outputs.genquery_output_files,
            [
                f"@@//buildfiles_genquery:fake_target.buildfiles.txt,{expected_gq}"
            ],
        )

    def test_parse_build_event_manifests_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            bep_path = Path(tmpdir) / "bep.json"
            bep_path.write_text(
                json.dumps(
                    {
                        "id": {"namedSet": {"id": "set-1"}},
                        "namedSetOfFiles": {
                            "files": [
                                {
                                    "name": "fake/target.fuchsia_source_files.json",
                                    "pathPrefix": ["fake-out", "bin"],
                                }
                            ]
                        },
                    }
                )
                + "\n"
                + json.dumps(
                    {
                        "id": {"targetCompleted": {"label": "//fake:target"}},
                        "completed": {
                            "outputGroup": [
                                {
                                    "name": "fuchsia_sources_manifest",
                                    "fileSets": [{"id": "set-1"}],
                                }
                            ]
                        },
                    }
                )
                + "\n"
            )
            outputs = parse_build_event_manifests(bep_path)
            expected = os.path.join(
                "fake-out", "bin", "fake/target.fuchsia_source_files.json"
            )
            self.assertEqual(outputs.source_files_manifest_paths, [expected])


if __name__ == "__main__":
    unittest.main()
