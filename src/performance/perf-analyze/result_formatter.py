# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

"""Formatting utilities for query results and errors."""

import csv
import io
import json
from abc import ABC, abstractmethod
from typing import Any


class Formatter(ABC):
    """Base class for result formatters."""

    @abstractmethod
    def format_results(self, data: list[dict[str, Any]]) -> str:
        """Formats query results as a string."""

    @abstractmethod
    def format_error(self, error_msg: str) -> str:
        """Formats an error message as a string."""


class JsonFormatter(Formatter):
    """Formats results and errors as JSON."""

    def format_results(self, data: list[dict[str, Any]]) -> str:
        return json.dumps(data, indent=2)

    def format_error(self, error_msg: str) -> str:
        return json.dumps({"error": error_msg}, indent=2)


def _is_batch(data: list[dict[str, Any]]) -> bool:
    """Checks if the data represents batch query/analysis results."""
    return bool(
        data
        and all(
            isinstance(x, dict)
            and "name" in x
            and ("results" in x or "error" in x)
            for x in data
        )
    )


class MarkdownFormatter(Formatter):
    """Formats results and errors as Markdown."""

    def format_results(self, data: list[dict[str, Any]]) -> str:
        if _is_batch(data):
            output = []
            for item in data:
                output.append(f"### {item['name']}")
                if "error" in item:
                    output.append(f"Error: {item['error']}")
                else:
                    results = item.get("results", [])
                    output.append(_table_to_markdown(results))
                output.append("")
            return "\n".join(output).strip()
        return _table_to_markdown(data)

    def format_error(self, error_msg: str) -> str:
        return f"**Error:** {error_msg}"


class TextFormatter(Formatter):
    """Formats results and errors as plain text (TSV)."""

    def format_results(self, data: list[dict[str, Any]]) -> str:
        if _is_batch(data):
            output = []
            for item in data:
                output.append(f"Query: {item['name']}")
                if "error" in item:
                    output.append(f"Error: {item['error']}")
                else:
                    results = item.get("results", [])
                    output.append(_table_to_text(results))
                output.append("")
            return "\n".join(output).strip()
        return _table_to_text(data)

    def format_error(self, error_msg: str) -> str:
        return f"Error: {error_msg}"


def _table_to_markdown(rows: list[dict[str, Any]]) -> str:
    """Formats a list of dicts as a Markdown table, escaping pipes."""
    if not rows:
        return "No results."
    raw_headers = list(rows[0].keys())
    headers = [h.replace("|", "\\|") for h in raw_headers]
    header_line = f"| {' | '.join(headers)} |"
    separator_line = f"| {' | '.join(['---'] * len(headers))} |"
    row_lines = []
    for row in rows:
        row_values = []
        for h in raw_headers:
            val_str = str(row.get(h, ""))
            val_str = val_str.replace("\r\n", "<br>").replace("\n", "<br>")
            row_values.append(val_str.replace("|", "\\|"))
        row_line = f"| {' | '.join(row_values)} |"
        row_lines.append(row_line)
    return "\n".join([header_line, separator_line] + row_lines)


def _table_to_text(rows: list[dict[str, Any]]) -> str:
    """Formats a list of dicts as TSV text using standard csv module."""
    if not rows:
        return "No results."
    headers = list(rows[0].keys())
    output = io.StringIO()
    writer = csv.writer(output, delimiter="\t", lineterminator="\n")
    writer.writerow(headers)
    for row in rows:
        writer.writerow([row.get(h, "") for h in headers])
    return output.getvalue().strip()


FORMATTERS: dict[str, Formatter] = {
    "json": JsonFormatter(),
    "markdown": MarkdownFormatter(),
    "text": TextFormatter(),
}
