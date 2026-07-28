#!/usr/bin/env fuchsia-vendored-python
# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

"""Fuchsia Standalone Performance Analysis Tool.

This tool provides CLI commands to query diagnostic plugins and analyze performance.
"""

import argparse
import sys
from typing import Sequence


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
    subparsers = parser.add_subparsers(dest="command")

    # 'query' subcommand
    query_parser = subparsers.add_parser(
        "query", help="Query diagnostic plugins"
    )
    query_parser.add_argument(
        "--list-plugins", action="store_true", help="List available plugins"
    )

    args = parser.parse_args(argv)

    if args.command is None:
        parser.print_help()
        return 2

    if args.command == "query":
        if args.list_plugins:
            print("No plugins configured.")
            return 0
        else:
            query_parser.print_help()
            return 2

    return 0


if __name__ == "__main__":
    sys.exit(main())
