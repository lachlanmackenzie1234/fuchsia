# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.
"""Unit tests for the result_formatter module."""
import unittest
from typing import Any

from result_formatter import FORMATTERS


class ResultFormatterTest(unittest.TestCase):
    """Tests for result formatters."""

    def test_format_results(self) -> None:
        """Tests format_results with normal data."""
        test_cases: list[tuple[str, list[dict[str, Any]], str]] = [
            (
                "json",
                [{"col1": "val1", "col2": 2}],
                '[\n  {\n    "col1": "val1",\n    "col2": 2\n  }\n]',
            ),
            (
                "markdown",
                [{"col1": "val1", "col2": 2}],
                "| col1 | col2 |\n| --- | --- |\n| val1 | 2 |",
            ),
            (
                "text",
                [{"col1": "val1", "col2": 2}],
                "col1\tcol2\nval1\t2",
            ),
        ]
        for fmt_name, data, expected in test_cases:
            with self.subTest(format=fmt_name):
                formatter = FORMATTERS[fmt_name]
                output = formatter.format_results(data)
                self.assertEqual(output.strip(), expected.strip())

    def test_format_results_empty(self) -> None:
        """Tests format_results with empty data."""
        test_cases: list[tuple[str, list[dict[str, Any]], str]] = [
            ("json", [], "[]"),
            ("markdown", [], "No results."),
            ("text", [], "No results."),
        ]
        for fmt_name, data, expected in test_cases:
            with self.subTest(format=fmt_name):
                formatter = FORMATTERS[fmt_name]
                output = formatter.format_results(data)
                self.assertEqual(output.strip(), expected.strip())

    def test_format_results_newlines(self) -> None:
        """Tests format_results with newlines in data."""
        test_cases: list[tuple[str, list[dict[str, Any]], str]] = [
            (
                "json",
                [{"col1": "line1\nline2"}],
                '[\n  {\n    "col1": "line1\\nline2"\n  }\n]',
            ),
            (
                "markdown",
                [{"col1": "line1\nline2"}],
                "| col1 |\n| --- |\n| line1<br>line2 |",
            ),
            (
                "text",
                [{"col1": "line1\nline2"}],
                'col1\n"line1\nline2"',
            ),
        ]
        for fmt_name, data, expected in test_cases:
            with self.subTest(format=fmt_name):
                formatter = FORMATTERS[fmt_name]
                output = formatter.format_results(data)
                self.assertEqual(output.strip(), expected.strip())

    def test_format_results_batch(self) -> None:
        """Tests format_results with batch data."""
        batch_data: list[dict[str, Any]] = [
            {
                "name": "Query 1",
                "results": [{"col1": "val1"}],
            },
            {
                "name": "Query 2",
                "error": "Some error occurred",
            },
        ]
        test_cases: list[tuple[str, str]] = [
            (
                "json",
                '[\n  {\n    "name": "Query 1",\n    "results": [\n      {\n        "col1": "val1"\n      }\n    ]\n  },\n  {\n    "name": "Query 2",\n    "error": "Some error occurred"\n  }\n]',
            ),
            (
                "markdown",
                "### Query 1\n| col1 |\n| --- |\n| val1 |\n\n### Query 2\nError: Some error occurred",
            ),
            (
                "text",
                "Query: Query 1\ncol1\nval1\n\nQuery: Query 2\nError: Some error occurred",
            ),
        ]
        for fmt_name, expected in test_cases:
            with self.subTest(format=fmt_name):
                formatter = FORMATTERS[fmt_name]
                output = formatter.format_results(batch_data)
                self.assertEqual(output.strip(), expected.strip())

    def test_format_error(self) -> None:
        """Tests format_error."""
        test_cases: list[tuple[str, str, str]] = [
            ("json", "some error", '{\n  "error": "some error"\n}'),
            ("markdown", "some error", "**Error:** some error"),
            ("text", "some error", "Error: some error"),
        ]
        for fmt_name, error_msg, expected in test_cases:
            with self.subTest(format=fmt_name):
                formatter = FORMATTERS[fmt_name]
                output = formatter.format_error(error_msg)
                self.assertEqual(output.strip(), expected.strip())


if __name__ == "__main__":
    unittest.main()
