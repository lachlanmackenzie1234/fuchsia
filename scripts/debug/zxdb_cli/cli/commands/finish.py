# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

from typing import Any

from cli.commands.base import BaseCommand


class Command(BaseCommand):
    COMMAND_NAME = "finish"

    @staticmethod
    def register_cli(subparsers: Any) -> None:
        parser = subparsers.add_parser(
            "finish",
            aliases=["step-out", "step_out", "stepout"],
            help="Finish execution of current function (step out)",
        )
        parser.add_argument("thread_id", type=int, help="Thread ID to finish")
        parser.add_argument(
            "--single-thread",
            action="store_true",
            default=None,
            help="Resume only the specified thread during step out",
        )
