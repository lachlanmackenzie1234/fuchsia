# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

"""Unit tests for the perf-analyze tool."""

import importlib.resources
import io
import json
import unittest
from contextlib import redirect_stderr, redirect_stdout

from perf_analyze import main


class PerfAnalyzeTest(unittest.TestCase):
    """Tests for the perf-analyze command line tool."""

    def test_query_sql_text(self) -> None:
        """Tests executing a simple SQL query in default text format."""
        source = importlib.resources.files("test_data").joinpath(
            "sample_fxt.fxt"
        )
        with importlib.resources.as_file(source) as trace_path:
            stdout = io.StringIO()
            with redirect_stdout(stdout):
                exit_code = main(
                    [
                        "query",
                        "--trace",
                        str(trace_path),
                        "--sql",
                        "select count(*) as cnt from slice",
                    ]
                )
            self.assertEqual(exit_code, 0)
            output = stdout.getvalue().strip()
            self.assertEqual(output, "cnt\n520")

    def test_query_sql_error(self) -> None:
        """Tests that query errors are formatted correctly."""
        source = importlib.resources.files("test_data").joinpath(
            "sample_fxt.fxt"
        )
        with importlib.resources.as_file(source) as trace_path:
            stdout = io.StringIO()
            stderr = io.StringIO()
            with redirect_stdout(stdout), redirect_stderr(stderr):
                exit_code = main(
                    [
                        "--format",
                        "text",
                        "query",
                        "--trace",
                        str(trace_path),
                        "--sql",
                        "select invalid-sql from slice",
                    ]
                )
            self.assertNotEqual(exit_code, 0)
            output = stdout.getvalue().strip()
            self.assertTrue(
                output.startswith("Error: Traceback"), f"Output was: {output!r}"
            )
            self.assertIn("no such column: invalid", output)

    def test_batch_query(self) -> None:
        """Tests executing a batch of queries from a file."""
        trace_source = importlib.resources.files("test_data").joinpath(
            "sample_fxt.fxt"
        )
        queries_source = importlib.resources.files("test_data").joinpath(
            "sample_queries.json"
        )

        with importlib.resources.as_file(
            trace_source
        ) as trace_path, importlib.resources.as_file(
            queries_source
        ) as queries_path:
            stdout = io.StringIO()
            with redirect_stdout(stdout):
                exit_code = main(
                    [
                        "--format",
                        "json",
                        "query",
                        "--trace",
                        str(trace_path),
                        "--batch",
                        f"@{queries_path}",
                    ]
                )
            self.assertEqual(exit_code, 0)
            results = json.loads(stdout.getvalue())

            expected = [
                {"name": "slice_count", "results": [{"cnt": 520}]},
                {"name": "process_count", "results": [{"cnt": 2}]},
            ]
            self.assertEqual(results, expected)

    def test_query_help(self) -> None:
        """Tests that query --help displays query help."""
        stdout = io.StringIO()
        with redirect_stdout(stdout):
            exit_code = main(["query", "--help"])
        self.assertEqual(exit_code, 0)
        output = stdout.getvalue()
        self.assertIn("--trace", output)
        self.assertIn("--sql", output)
        self.assertIn("--batch", output)

    def test_query_empty_results_text(self) -> None:
        """Tests that queries with no results format as 'No results.'"""
        source = importlib.resources.files("test_data").joinpath(
            "sample_fxt.fxt"
        )
        with importlib.resources.as_file(source) as trace_path:
            stdout = io.StringIO()
            with redirect_stdout(stdout):
                exit_code = main(
                    [
                        "query",
                        "--trace",
                        str(trace_path),
                        "--sql",
                        "select * from slice where 1=0",
                    ]
                )
            self.assertEqual(exit_code, 0)
            output = stdout.getvalue().strip()
            self.assertEqual(output, "No results.")


if __name__ == "__main__":
    unittest.main()
