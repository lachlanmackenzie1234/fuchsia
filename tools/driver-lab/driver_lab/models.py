# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

"""Frozen data models for the driver-lab host tooling.

Models are immutable and validated at construction. Conversions to and
from serialized forms happen in explicit functions; unvalidated
dictionaries are never passed through the system.
"""

from __future__ import annotations

import dataclasses
import enum
import hashlib


class AccessClass(enum.Enum):
    """Access classes; a grant for one class never authorizes another."""

    READ_ONCE = "read_once"
    SNAPSHOT = "snapshot"
    POLL = "poll"
    WRITE = "write"
    PROTOCOL_TRANSACTION = "protocol_transaction"


class Decision(enum.Enum):
    """Persistent grant decision."""

    ALLOW = "allow"
    DENY = "deny"


def _check_common(resource_digest: str, offset: int, width: int) -> None:
    if not resource_digest.startswith("sha256:"):
        raise ValueError("resource_digest must be a sha256:<hex> digest")
    if offset < 0:
        raise ValueError("offset must be non-negative")
    if width <= 0:
        raise ValueError("width must be positive")


@dataclasses.dataclass(frozen=True)
class AccessRequest:
    """One register access whose consent must be resolved."""

    target_scope: str
    node_id: str
    resource_digest: str
    resource: str
    offset: int
    width: int
    access: AccessClass

    def __post_init__(self) -> None:
        _check_common(self.resource_digest, self.offset, self.width)

    @property
    def match_key(self) -> tuple[str, str, str, str, int, int, AccessClass]:
        """The exact identity a grant must match."""
        return (
            self.target_scope,
            self.node_id,
            self.resource_digest,
            self.resource,
            self.offset,
            self.width,
            self.access,
        )


@dataclasses.dataclass(frozen=True)
class ReadGrant:
    """A persistent operator consent rule.

    A grant matches on stable identity criteria only: target scope, node
    identity criteria, resource-description digest, logical resource,
    offset, width, and access class. It never matches solely by nodename,
    boot ID, or devfs path. A boot change alone does not invalidate a
    grant (no boot identity is recorded); a resource-description or
    per-resource policy change does, because the digest changes.
    """

    schema_version: int
    target_scope: str
    node_id: str
    resource_digest: str
    resource: str
    offset: int
    width: int
    access: AccessClass
    decision: Decision
    approved_at: str
    approval_source: str | None = None
    reason: str | None = None
    max_poll_hz: float | None = None
    max_poll_timeout_s: float | None = None

    def __post_init__(self) -> None:
        if self.schema_version != 1:
            raise ValueError(
                f"unsupported grant schema version {self.schema_version}"
            )
        _check_common(self.resource_digest, self.offset, self.width)
        if self.access is AccessClass.WRITE:
            raise ValueError("persistent write grants are not supported")
        has_poll_limits = (
            self.max_poll_hz is not None or self.max_poll_timeout_s is not None
        )
        if self.access is not AccessClass.POLL and has_poll_limits:
            raise ValueError("poll limits are only valid on poll grants")
        if (
            self.access is AccessClass.POLL
            and self.decision is Decision.ALLOW
            and (self.max_poll_hz is None or self.max_poll_timeout_s is None)
        ):
            raise ValueError(
                "a poll allow grant requires max_poll_hz and max_poll_timeout_s"
            )

    @property
    def match_key(self) -> tuple[str, str, str, str, int, int, AccessClass]:
        """The exact identity this grant matches."""
        return (
            self.target_scope,
            self.node_id,
            self.resource_digest,
            self.resource,
            self.offset,
            self.width,
            self.access,
        )

    @property
    def grant_id(self) -> str:
        """Stable identifier for list/explain/revoke operations."""
        material = "|".join(
            [
                self.target_scope,
                self.node_id,
                self.resource_digest,
                self.resource,
                f"{self.offset:#x}",
                str(self.width),
                self.access.value,
                self.decision.value,
            ]
        )
        return "grant-" + hashlib.sha256(material.encode()).hexdigest()[:16]

    def matches(self, request: AccessRequest) -> bool:
        """Whether this grant exactly matches `request`."""
        return self.match_key == request.match_key
