# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

from typing import Any

from cli.commands.base import BaseCommand


class Command(BaseCommand):
    """CLI command implementation for stepping over execution (next)."""

    COMMAND_NAME = "next"

    @staticmethod
    def register_cli(subparsers: Any) -> None:
        parser = subparsers.add_parser(
            "next",
            aliases=["n", "step-over", "step_over", "stepover"],
            help="Step over execution to the next line (next)",
        )
        parser.add_argument(
            "thread_id", type=int, help="Thread ID to step over"
        )
        parser.add_argument(
            "--single-thread",
            action="store_true",
            default=None,
            help="Resume only the specified thread during step over",
        )
        parser.add_argument(
            "--granularity",
            choices=["statement", "line", "instruction"],
            default=None,
            help="Stepping granularity",
        )
