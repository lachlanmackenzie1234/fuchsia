#!/usr/bin/env fuchsia-vendored-python
# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

"""Fuchsia Standalone Performance Analysis Tool.

This tool provides CLI commands to query diagnostic plugins and analyze performance.
"""

import argparse
import logging
import sys
from typing import Sequence

import result_formatter
from query import TpShell


def main(argv: Sequence[str] | None = None) -> int:
    """Main entry point for the performance analysis tool.

    Args:
        argv: Optional list of arguments to parse. Defaults to sys.argv[1:].

    Returns:
        Exit code (0 for success, non-zero for failure).
    """
    parser = argparse.ArgumentParser(
        description="Fuchsia Standalone Performance Analysis Tool"
    )
    parser.add_argument(
        "--format",
        choices=["json", "markdown", "text"],
        default="text",
        help="Output format (default: text)",
    )
    parser.add_argument(
        "--verbose",
        action="store_true",
        help="Enable verbose debug logging",
    )
    parser.add_argument(
        "--no-cache",
        action="store_false",
        dest="cache",
        help="Disable local caching of permalink URL traces",
    )
    subparsers = parser.add_subparsers(dest="command")

    # 'query' subcommand
    query_parser = subparsers.add_parser(
        "query", help="Perform SQL queries on a trace file"
    )
    query_parser.add_argument(
        "--trace", required=True, help="Trace file path or URL"
    )
    group = query_parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--sql", help="SQL query to run")
    group.add_argument("--batch", help="Batch JSON or @file")

    try:
        args = parser.parse_args(argv)
    except SystemExit as e:
        if e.code is None:
            return 0
        if isinstance(e.code, int):
            return e.code
        print(f"Error: {e.code}", file=sys.stderr)
        return 1

    # Configure logging based on verbose flag
    log_level = logging.DEBUG if args.verbose else logging.WARNING
    logging.basicConfig(
        level=log_level,
        format="%(levelname)s:%(name)s:%(message)s",
        stream=sys.stderr,
    )

    if args.command is None:
        parser.print_help()
        return 2

    if args.command == "query":
        tp_shell = TpShell()
        formatter = result_formatter.FORMATTERS[args.format]
        try:
            results = tp_shell.query(
                trace_path=args.trace,
                sql=args.sql,
                batch=args.batch,
                cache=args.cache,
            )
            # Format and print results
            print(formatter.format_results(results))
            return 0
        except Exception as e:
            print(formatter.format_error(str(e)))
            return 1

    return 0


if __name__ == "__main__":
    sys.exit(main())
