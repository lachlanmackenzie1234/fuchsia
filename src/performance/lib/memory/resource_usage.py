# Copyright 2025 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

import gc
import resource
import sys
from types import FunctionType, ModuleType
from typing import Any, NamedTuple


class MaxRss(NamedTuple):
    self_bytes: int
    children_bytes: int


# Referents of Function objects include the modules they're in, which isn't
# really the kind of "size" we're trying to get at here. Module and type objects
# have similar issues.
_DENYLIST = type, ModuleType, FunctionType

# Built-in leaf types that never hold outgoing Python object references in
# CPython. Checked with exact type identity (`type(obj) in _ATOMIC_TYPES`) so
# user subclasses (e.g. IntEnum or custom int/str subclasses with __dict__)
# still have their referents traversed.
_ATOMIC_TYPES = frozenset(
    {
        int,
        str,
        float,
        bytes,
        bool,
        type(None),
        complex,
        bytearray,
        range,
    }
)


def get_deep_size(obj: Any) -> int:
    """Traverse obj and its members, summing their sizes."""
    if isinstance(obj, _DENYLIST):
        raise TypeError(
            f"get_deep_size() does not take argument of type: {str(type(obj))}"
        )
    if type(obj) in _ATOMIC_TYPES:
        return sys.getsizeof(obj)

    seen_ids: set[int] = {id(obj)}
    size = sys.getsizeof(obj)
    stack: list[Any] = [obj]
    while stack:
        current = stack.pop()
        for ref in gc.get_referents(current):
            oid = id(ref)
            if oid not in seen_ids:
                seen_ids.add(oid)
                if type(ref) in _ATOMIC_TYPES:
                    size += sys.getsizeof(ref)
                elif not isinstance(ref, _DENYLIST):
                    size += sys.getsizeof(ref)
                    stack.append(ref)
    return size


def get_max_rss_bytes() -> MaxRss:
    """Returns the maximum resident set size of the current process tree in bytes."""
    return MaxRss(
        self_bytes=_get_current_process_max_rss_kb() * 1024,
        children_bytes=_get_child_processes_max_rss_kb() * 1024,
    )


def _get_current_process_max_rss_kb() -> int:
    """Returns the maximum resident set size of the current process in kilobytes."""
    return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss


def _get_child_processes_max_rss_kb() -> int:
    """
    Returns the maximum resident set size of all terminated child processes
    in kilobytes.
    """
    return resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
