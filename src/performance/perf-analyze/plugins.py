# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

"""Diagnostic analysis plugins for perf-analyze."""

import argparse
import io
from typing import Any, NoReturn, Protocol, Sequence, runtime_checkable


class PluginArgumentError(Exception):
    """Exception raised when plugin arguments are invalid."""


class PluginArgumentParser(argparse.ArgumentParser):
    """Custom ArgumentParser that raises PluginArgumentError instead of calling sys.exit."""

    def error(self, message: str) -> NoReturn:
        fp = io.StringIO()
        self.print_usage(fp)
        usage = fp.getvalue().strip()
        raise PluginArgumentError(f"{usage}\nError: {message}")


@runtime_checkable
class AnalyzePlugin(Protocol):
    """Protocol defining the interface for performance analysis plugins."""

    name: str
    description: str

    def analyze(
        self,
        remaining_args: Sequence[str],
        trace_path: str,
        cache: bool = True,
    ) -> list[dict[str, Any]]:
        """Executes the analysis specified by args on the given trace."""
        ...
