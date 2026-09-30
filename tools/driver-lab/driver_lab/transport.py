# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

"""Wire-contract-shaped transport abstraction.

Mirrors the `fuchsia.driver.lab` wire contract in transport-neutral
types. Real implementations speak FIDL through fuchsia-controller;
`FakeProxyTarget` implements the same contract for host tests, including
staleness rejection, exact-allowlist enforcement, and audit sequencing.
"""

from __future__ import annotations

import dataclasses
import enum
from collections.abc import Sequence
from typing import Protocol

from driver_lab.models import AccessClass


class Denial(enum.Enum):
    """Machine-readable operation refusal reasons (wire `OperationError`)."""

    UNKNOWN_RESOURCE = "unknown_resource"
    UNSUPPORTED_WIDTH = "unsupported_width"
    UNSUPPORTED_ACCESS_CLASS = "unsupported_access_class"
    MISALIGNED = "misaligned"
    OFFSET_OVERFLOW = "offset_overflow"
    OUT_OF_LOGICAL_BOUNDS = "out_of_logical_bounds"
    OUT_OF_MAPPED_BOUNDS = "out_of_mapped_bounds"
    NOT_PERMITTED_BY_CEILING = "not_permitted_by_ceiling"
    HARD_DENIED = "hard_denied"
    UNKNOWN_READS_NOT_PERMITTED = "unknown_reads_not_permitted"
    NOT_IN_ALLOWLIST = "not_in_allowlist"
    LIMIT_EXCEEDED = "limit_exceeded"
    STALE_IDENTITY = "stale_identity"
    MUTATION_LEASE_CONTENTION = "mutation_lease_contention"
    NOT_ACCEPTING = "not_accepting"
    BACKEND_FAULT = "backend_fault"
    UNSUPPORTED_EXPECTATION = "unsupported_expectation"


class OpenRejection(enum.Enum):
    """Machine-readable session-open refusals (wire `OpenSessionError`)."""

    NOT_ACCEPTING = "not_accepting"
    STALE_BOOT_ID = "stale_boot_id"
    STALE_PROXY_GENERATION = "stale_proxy_generation"
    STALE_RESOURCE_DIGEST = "stale_resource_digest"
    STALE_POLICY_DIGEST = "stale_policy_digest"
    REJECTED_ALLOWLIST = "rejected_allowlist"
    MUTATION_LEASE_HELD = "mutation_lease_held"
    UNSUPPORTED_EXPECTATION = "unsupported_expectation"


STALE_REJECTIONS = frozenset(
    {
        OpenRejection.STALE_BOOT_ID,
        OpenRejection.STALE_PROXY_GENERATION,
        OpenRejection.STALE_RESOURCE_DIGEST,
        OpenRejection.STALE_POLICY_DIGEST,
    }
)


class TransportError(Exception):
    """A transport-level failure."""


class OperationDenied(TransportError):
    """The target refused an operation; the audit ring has the record."""

    def __init__(self, denial: Denial) -> None:
        super().__init__(denial.value)
        self.denial = denial


class OpenSessionRejected(TransportError):
    """The target refused to open a session; no hardware was touched."""

    def __init__(self, reason: OpenRejection) -> None:
        super().__init__(reason.value)
        self.reason = reason


@dataclasses.dataclass(frozen=True)
class ResourceInfo:
    """One offered resource, as reported by `Describe`."""

    id: int
    name: str
    logical_size: int
    digest: str


@dataclasses.dataclass(frozen=True)
class Expectations:
    """Expected proxy identity echoed back when opening a session."""

    boot_id: str
    proxy_generation: int
    resource_digest: str
    policy_digest: str


@dataclasses.dataclass(frozen=True)
class ProxyDescription:
    """Identity, resources, and limits of one proxy instance."""

    protocol_major: int
    protocol_minor: int
    proxy_generation: int
    boot_id: str
    resource_digest: str
    policy_digest: str
    resources: tuple[ResourceInfo, ...]
    max_snapshot_items: int
    audit_capacity: int
    # Absent until the proxy binds to a real node.
    node_moniker: str | None = None
    # Reserved for phase 2 (managed takeover).
    topology_generation: int | None = None
    # Takeover lifecycle state; phase 1 proxies report "not_active".
    takeover: str | None = None

    def expectations(self) -> Expectations:
        """The expectations a fresh describe implies."""
        return Expectations(
            boot_id=self.boot_id,
            proxy_generation=self.proxy_generation,
            resource_digest=self.resource_digest,
            policy_digest=self.policy_digest,
        )

    def resource_named(self, name: str) -> ResourceInfo | None:
        """The resource with logical name `name`, if offered."""
        for resource in self.resources:
            if resource.name == name:
                return resource
        return None


@dataclasses.dataclass(frozen=True)
class SessionContext:
    """Host-supplied provenance recorded with the session."""

    run_id: str
    case_id: str
    plan_digest: str
    host_tool_version: str = "driver-lab-dev"


@dataclasses.dataclass(frozen=True)
class AllowRule:
    """One exact session allowlist rule."""

    resource: int
    offset: int
    width: int
    access: AccessClass


@dataclasses.dataclass(frozen=True)
class ReadOutcome:
    """A completed 32-bit read."""

    value: int
    audit_seq: int
    timestamp_ns: int


@dataclasses.dataclass(frozen=True)
class SnapshotItem:
    """One requested snapshot read."""

    resource: int
    offset: int


@dataclasses.dataclass(frozen=True)
class SnapshotItemOutcome:
    """The result of one snapshot item that began execution."""

    item: SnapshotItem
    ok: bool
    value: int
    audit_seq: int
    timestamp_ns: int


@dataclasses.dataclass(frozen=True)
class SnapshotOutcome:
    """A snapshot whose prevalidation succeeded; never atomic."""

    results: tuple[SnapshotItemOutcome, ...]
    complete: bool


@dataclasses.dataclass(frozen=True)
class AuditEntry:
    """One target audit record."""

    seq: int
    operation: str
    session: int | None = None
    resource: int | None = None
    offset: int | None = None
    decision: str = "allowed"
    denial: str | None = None
    status: str = "ok"
    value: int | None = None
    timestamp_ns: int = 0
    run_id: str | None = None
    item_index: int | None = None
    proxy_generation: int | None = None
    boot_id: str | None = None

    def to_json(self) -> dict[str, object]:
        """A JSON-serializable row for evidence."""
        return dataclasses.asdict(self)


@dataclasses.dataclass(frozen=True)
class AuditPage:
    """One bounded audit page."""

    entries: tuple[AuditEntry, ...]
    oldest_retained: int | None
    next_cursor: int


class ProxySession(Protocol):
    """One open experiment session."""

    async def read32(self, resource: int, offset: int) -> ReadOutcome:
        """One policy-checked, audited 32-bit read."""
        ...

    async def snapshot(self, items: Sequence[SnapshotItem]) -> SnapshotOutcome:
        """A bounded snapshot; every item validated before the first read."""
        ...

    async def read_audit(self, cursor: int, limit: int) -> AuditPage:
        """Reads a bounded audit page for incremental draining."""
        ...

    async def close(self) -> None:
        """Closes the session and releases its leases."""
        ...


class ProxyTransport(Protocol):
    """A connection to one proxy target."""

    async def describe(self) -> ProxyDescription:
        """The proxy's identity, resources, and limits."""
        ...

    async def open_session(
        self,
        context: SessionContext,
        expectations: Expectations,
        allowlist: Sequence[AllowRule],
    ) -> ProxySession:
        """Opens a session with an exact allowlist."""
        ...


class FakeProxyTarget:
    """In-memory proxy target honoring the wire-contract semantics."""

    def __init__(self, description: ProxyDescription) -> None:
        self._description = description
        self._values: dict[tuple[int, int], int] = {}
        self._faults: set[tuple[int, int]] = set()
        self._audit: list[AuditEntry] = []
        self._next_seq = 0
        self._now_ns = 1000
        self._next_session = 1
        self.open_attempts = 0
        self.sessions_opened = 0
        self.reject_open: OpenRejection | None = None

    def set_value(self, resource: int, offset: int, value: int) -> None:
        """Programs the value returned for reads at (resource, offset)."""
        self._values[(resource, offset)] = value

    def fail_at(self, resource: int, offset: int) -> None:
        """Makes reads at (resource, offset) fail as a backend fault."""
        self._faults.add((resource, offset))

    def _tick(self) -> int:
        self._now_ns += 10
        return self._now_ns

    def _append(self, entry_fields: dict[str, object]) -> int:
        seq = self._next_seq
        self._next_seq += 1
        # Instance identity is stamped on every drained entry, matching
        # the real proxy's drain-time behavior.
        entry_fields.setdefault(
            "proxy_generation", self._description.proxy_generation
        )
        entry_fields.setdefault("boot_id", self._description.boot_id)
        self._audit.append(AuditEntry(seq=seq, **entry_fields))  # type: ignore[arg-type]
        return seq

    async def describe(self) -> ProxyDescription:
        """See `ProxyTransport.describe`."""
        return self._description

    async def open_session(
        self,
        context: SessionContext,
        expectations: Expectations,
        allowlist: Sequence[AllowRule],
    ) -> ProxySession:
        """See `ProxyTransport.open_session`; enforces staleness checks."""
        self.open_attempts += 1
        description = self._description
        rejection: OpenRejection | None = self.reject_open
        if rejection is None:
            if expectations.boot_id != description.boot_id:
                rejection = OpenRejection.STALE_BOOT_ID
            elif expectations.proxy_generation != description.proxy_generation:
                rejection = OpenRejection.STALE_PROXY_GENERATION
            elif expectations.resource_digest != description.resource_digest:
                rejection = OpenRejection.STALE_RESOURCE_DIGEST
            elif expectations.policy_digest != description.policy_digest:
                rejection = OpenRejection.STALE_POLICY_DIGEST
        if rejection is None:
            known = {resource.id for resource in description.resources}
            if any(
                rule.resource not in known or rule.width != 4
                for rule in allowlist
            ):
                rejection = OpenRejection.REJECTED_ALLOWLIST
        if rejection is not None:
            self._append(
                {
                    "operation": "open_session",
                    "decision": "denied",
                    "denial": rejection.value,
                    "status": "rejected",
                    "timestamp_ns": self._tick(),
                    "run_id": context.run_id,
                }
            )
            raise OpenSessionRejected(rejection)
        session_id = self._next_session
        self._next_session += 1
        self.sessions_opened += 1
        self._append(
            {
                "operation": "open_session",
                "session": session_id,
                "timestamp_ns": self._tick(),
                "run_id": context.run_id,
            }
        )
        rules = {
            (rule.resource, rule.offset, rule.access) for rule in allowlist
        }
        return _FakeSession(self, session_id, rules)


class _FakeSession:
    def __init__(
        self,
        target: FakeProxyTarget,
        session_id: int,
        rules: set[tuple[int, int, AccessClass]],
    ) -> None:
        self._target = target
        self._session = session_id
        self._rules = rules

    def _deny(
        self, operation: str, resource: int, offset: int, denial: Denial
    ) -> None:
        target = self._target
        target._append(
            {
                "operation": operation,
                "session": self._session,
                "resource": resource,
                "offset": offset,
                "decision": "denied",
                "denial": denial.value,
                "status": "rejected",
                "timestamp_ns": target._tick(),
            }
        )
        raise OperationDenied(denial)

    def _read(
        self, operation: str, resource: int, offset: int, access: AccessClass
    ) -> ReadOutcome:
        target = self._target
        if all(info.id != resource for info in target._description.resources):
            self._deny(operation, resource, offset, Denial.UNKNOWN_RESOURCE)
        if (resource, offset, access) not in self._rules:
            self._deny(operation, resource, offset, Denial.NOT_IN_ALLOWLIST)
        timestamp = target._tick()
        if (resource, offset) in target._faults:
            target._append(
                {
                    "operation": operation,
                    "session": self._session,
                    "resource": resource,
                    "offset": offset,
                    "status": "backend_fault",
                    "timestamp_ns": timestamp,
                }
            )
            raise OperationDenied(Denial.BACKEND_FAULT)
        value = target._values.get((resource, offset), 0)
        seq = target._append(
            {
                "operation": operation,
                "session": self._session,
                "resource": resource,
                "offset": offset,
                "value": value,
                "timestamp_ns": timestamp,
            }
        )
        return ReadOutcome(value=value, audit_seq=seq, timestamp_ns=timestamp)

    async def read32(self, resource: int, offset: int) -> ReadOutcome:
        """See `ProxySession.read32`."""
        return self._read("read32", resource, offset, AccessClass.READ_ONCE)

    async def snapshot(self, items: Sequence[SnapshotItem]) -> SnapshotOutcome:
        """See `ProxySession.snapshot`."""
        target = self._target
        if len(items) > target._description.max_snapshot_items:
            self._deny("snapshot", 0, 0, Denial.LIMIT_EXCEEDED)
        for item in items:
            if (
                item.resource,
                item.offset,
                AccessClass.SNAPSHOT,
            ) not in self._rules:
                self._deny(
                    "snapshot",
                    item.resource,
                    item.offset,
                    Denial.NOT_IN_ALLOWLIST,
                )
        results: list[SnapshotItemOutcome] = []
        complete = True
        for item in items:
            timestamp = target._tick()
            if (item.resource, item.offset) in target._faults:
                seq = target._append(
                    {
                        "operation": "snapshot_read32",
                        "session": self._session,
                        "resource": item.resource,
                        "offset": item.offset,
                        "status": "backend_fault",
                        "timestamp_ns": timestamp,
                    }
                )
                results.append(
                    SnapshotItemOutcome(
                        item=item,
                        ok=False,
                        value=0,
                        audit_seq=seq,
                        timestamp_ns=timestamp,
                    )
                )
                complete = False
                break
            value = target._values.get((item.resource, item.offset), 0)
            seq = target._append(
                {
                    "operation": "snapshot_read32",
                    "session": self._session,
                    "resource": item.resource,
                    "offset": item.offset,
                    "value": value,
                    "timestamp_ns": timestamp,
                }
            )
            results.append(
                SnapshotItemOutcome(
                    item=item,
                    ok=True,
                    value=value,
                    audit_seq=seq,
                    timestamp_ns=timestamp,
                )
            )
        return SnapshotOutcome(results=tuple(results), complete=complete)

    async def read_audit(self, cursor: int, limit: int) -> AuditPage:
        """See `ProxySession.read_audit`."""
        target = self._target
        entries = tuple(
            entry for entry in target._audit if entry.seq >= cursor
        )[:limit]
        oldest = target._audit[0].seq if target._audit else None
        next_cursor = entries[-1].seq + 1 if entries else cursor
        return AuditPage(
            entries=entries, oldest_retained=oldest, next_cursor=next_cursor
        )

    async def close(self) -> None:
        """See `ProxySession.close`."""
        target = self._target
        target._append(
            {
                "operation": "session_closed",
                "session": self._session,
                "timestamp_ns": target._tick(),
            }
        )
