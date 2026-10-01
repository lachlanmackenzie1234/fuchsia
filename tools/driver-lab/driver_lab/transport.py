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
from collections.abc import Callable, Mapping, Sequence
from typing import Protocol, runtime_checkable

from driver_lab.models import (
    AccessClass,
    GpioReadOutcome,
    GpioWriteOutcome,
    I2cTransferOutcome,
    ResourceKind,
    SpiTransmitOutcome,
)


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
    PRECONDITION_FAILED = "precondition_failed"
    MISSING_PRECONDITION = "missing_precondition"
    TIMEOUT = "timeout"
    READ_ONLY_SESSION = "read_only_session"
    WRITE_NOT_PERMITTED = "write_not_permitted"
    POLL_NOT_PERMITTED = "poll_not_permitted"
    UNSUPPORTED_METHOD = "unsupported_method"
    TRANSFER_TOO_LARGE = "transfer_too_large"


class SessionMode(enum.Enum):
    """Session access mode: read-only or mutating."""

    READ_ONLY = "read_only"
    MUTATING = "mutating"


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
    kind: ResourceKind = ResourceKind.MMIO


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
class WriteOutcome:
    """A completed 32-bit write."""

    readback_value: int | None
    audit_seq: int
    timestamp_ns: int


@dataclasses.dataclass(frozen=True)
class PollOutcome:
    """A completed 32-bit poll."""

    value: int
    audit_seq: int
    timestamp_ns: int


class BarrierVariant(enum.Enum):
    """Memory barrier variant."""

    MEMORY = "memory"


@dataclasses.dataclass(frozen=True)
class SequenceItem:
    """One item in an ordered sequence."""

    kind: str  # "mmio_read32", "mmio_write32", "mmio_poll32", "delay_ns", "barrier", "gpio_read", "gpio_write", "i2c_transfer", "spi_transmit"
    resource: int = 0
    offset: int = 0
    value: int = 0
    write_mask: int = 0xFFFF_FFFF
    precondition: tuple[int, int] | None = None  # (expected, mask)
    readback: bool = True
    expected: int = 0
    mask: int = 0xFFFF_FFFF
    interval_ns: int = 1_000_000
    timeout_ns: int = 100_000_000
    delay_ns: int = 0
    barrier: str = "memory"
    write_data: bytes = b""
    read_length: int = 0
    tx_data: bytes = b""


@dataclasses.dataclass(frozen=True)
class SequenceItemOutcome:
    """The outcome of one item in a sequence."""

    index: int
    ok: bool
    kind: str
    value: int | None = None
    readback_value: int | None = None
    audit_seq: int = 0
    timestamp_ns: int = 0
    data: bytes = b""
    error: Denial | None = None


@dataclasses.dataclass(frozen=True)
class SequenceOutcome:
    """An executed sequence of operations."""

    results: tuple[SequenceItemOutcome, ...]
    complete: bool


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


@runtime_checkable
class ProxySession(Protocol):
    """One open experiment session."""

    async def read32(self, resource: int, offset: int) -> ReadOutcome:
        """One policy-checked, audited 32-bit read."""
        ...

    async def snapshot(self, items: Sequence[SnapshotItem]) -> SnapshotOutcome:
        """A bounded snapshot; every item validated before the first read."""
        ...

    async def write32(
        self,
        resource: int,
        offset: int,
        value: int,
        write_mask: int = 0xFFFF_FFFF,
        precondition: tuple[int, int] | None = None,
        readback: bool = True,
    ) -> WriteOutcome:
        """One policy-checked, audited 32-bit write."""
        ...

    async def poll32(
        self,
        resource: int,
        offset: int,
        expected: int,
        mask: int = 0xFFFF_FFFF,
        interval_ns: int = 1_000_000,
        timeout_ns: int = 100_000_000,
    ) -> PollOutcome:
        """Repeated bounded read until match or timeout."""
        ...

    async def execute_sequence(
        self, items: Sequence[SequenceItem]
    ) -> SequenceOutcome:
        """Bounded ordered sequence of operations with prevalidation."""
        ...

    async def gpio_read(self, resource: int) -> GpioReadOutcome:
        """One policy-checked, audited GPIO read."""
        ...

    async def gpio_write(self, resource: int, value: bool) -> GpioWriteOutcome:
        """One policy-checked, audited GPIO write."""
        ...

    async def i2c_transfer(
        self, resource: int, write_data: bytes, read_length: int = 0
    ) -> I2cTransferOutcome:
        """One policy-checked, audited I2C transfer."""
        ...

    async def spi_transmit(
        self, resource: int, tx_data: bytes
    ) -> SpiTransmitOutcome:
        """One policy-checked, audited SPI transmit."""
        ...

    async def read_audit(self, cursor: int, limit: int) -> AuditPage:
        """Reads a bounded audit page for incremental draining."""
        ...

    async def close(self) -> None:
        """Closes the session and releases its leases."""
        ...


@runtime_checkable
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
        mode: SessionMode = SessionMode.READ_ONLY,
    ) -> ProxySession:
        """Opens a session with an exact allowlist."""
        ...


class FakeProxyTarget:
    """In-memory proxy target honoring the wire-contract semantics."""

    def __init__(self, description: ProxyDescription) -> None:
        self._description = description
        self._values: dict[tuple[int, int], int] = {}
        self._gpio_values: dict[int, bool] = {}
        self._i2c_responses: dict[int, bytes] = {}
        self._spi_responses: dict[int, bytes] = {}
        self._gpio_writes: list[tuple[int, bool]] = []
        self._i2c_transfers: list[tuple[int, bytes, int]] = []
        self._spi_transmits: list[tuple[int, bytes]] = []
        self._faults: set[tuple[int, int]] = set()
        self._audit: list[AuditEntry] = []
        self._next_seq = 0
        self._now_ns = 1000
        self._next_session = 1
        self.open_attempts = 0
        self.sessions_opened = 0
        self.reject_open: OpenRejection | None = None
        self.active_mutating_session: int | None = None

    def set_gpio(self, resource: int, value: bool) -> None:
        """Sets the state of a GPIO pin."""
        self._gpio_values[resource] = value

    def get_gpio(self, resource: int) -> bool:
        """Returns the state of a GPIO pin."""
        return self._gpio_values.get(resource, False)

    def set_i2c_response(self, resource: int, data: bytes) -> None:
        """Sets the read response returned for I2C transfers."""
        self._i2c_responses[resource] = data

    def set_spi_response(self, resource: int, data: bytes) -> None:
        """Sets the receive response returned for SPI transmits."""
        self._spi_responses[resource] = data

    def set_value(self, resource: int, offset: int, value: int) -> None:
        """Programs the value returned for reads at (resource, offset)."""
        self._values[(resource, offset)] = value

    def get_value(self, resource: int, offset: int) -> int:
        """Returns the current stored value at (resource, offset)."""
        return self._values.get((resource, offset), 0)

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
        mode: SessionMode = SessionMode.READ_ONLY,
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
            elif (
                mode == SessionMode.MUTATING
                and self.active_mutating_session is not None
            ):
                rejection = OpenRejection.MUTATION_LEASE_HELD
        if rejection is None:
            known = {
                resource.id: resource for resource in description.resources
            }
            for rule in allowlist:
                res = known.get(rule.resource)
                if res is None:
                    rejection = OpenRejection.REJECTED_ALLOWLIST
                    break
                if rule.access == AccessClass.PROTOCOL:
                    if res.kind == ResourceKind.MMIO:
                        rejection = OpenRejection.REJECTED_ALLOWLIST
                        break
                elif rule.access == AccessClass.SEQUENCE:
                    if res.kind == ResourceKind.MMIO and rule.width != 4:
                        rejection = OpenRejection.REJECTED_ALLOWLIST
                        break
                else:
                    if rule.width != 4:
                        rejection = OpenRejection.REJECTED_ALLOWLIST
                        break
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
        if mode == SessionMode.MUTATING:
            self.active_mutating_session = session_id
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
        return _FakeSession(self, session_id, rules, mode)


class _FakeSession:
    def __init__(
        self,
        target: FakeProxyTarget,
        session_id: int,
        rules: set[tuple[int, int, AccessClass]],
        mode: SessionMode = SessionMode.READ_ONLY,
    ) -> None:
        self._target = target
        self._session = session_id
        self._rules = rules
        self._mode = mode

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

    async def write32(
        self,
        resource: int,
        offset: int,
        value: int,
        write_mask: int = 0xFFFF_FFFF,
        precondition: tuple[int, int] | None = None,
        readback: bool = True,
    ) -> WriteOutcome:
        target = self._target
        if self._mode != SessionMode.MUTATING:
            self._deny("write32", resource, offset, Denial.READ_ONLY_SESSION)
        if all(info.id != resource for info in target._description.resources):
            self._deny("write32", resource, offset, Denial.UNKNOWN_RESOURCE)
        if (resource, offset, AccessClass.WRITE) not in self._rules:
            self._deny("write32", resource, offset, Denial.NOT_IN_ALLOWLIST)
        current = target._values.get((resource, offset), 0)
        if precondition is not None:
            expected, mask = precondition
            if (current & mask) != (expected & mask):
                self._deny(
                    "write32", resource, offset, Denial.PRECONDITION_FAILED
                )
        timestamp = target._tick()
        if (resource, offset) in target._faults:
            target._append(
                {
                    "operation": "write32",
                    "session": self._session,
                    "resource": resource,
                    "offset": offset,
                    "status": "backend_fault",
                    "timestamp_ns": timestamp,
                }
            )
            raise OperationDenied(Denial.BACKEND_FAULT)
        new_val = (current & ~write_mask) | (value & write_mask)
        target._values[(resource, offset)] = new_val
        rb_val = target._values.get((resource, offset), 0) if readback else None
        seq = target._append(
            {
                "operation": "write32",
                "session": self._session,
                "resource": resource,
                "offset": offset,
                "value": new_val,
                "timestamp_ns": timestamp,
            }
        )
        return WriteOutcome(
            readback_value=rb_val,
            audit_seq=seq,
            timestamp_ns=timestamp,
        )

    async def poll32(
        self,
        resource: int,
        offset: int,
        expected: int,
        mask: int = 0xFFFF_FFFF,
        interval_ns: int = 1_000_000,
        timeout_ns: int = 100_000_000,
    ) -> PollOutcome:
        target = self._target
        if all(info.id != resource for info in target._description.resources):
            self._deny("poll32", resource, offset, Denial.UNKNOWN_RESOURCE)
        if (resource, offset, AccessClass.POLL) not in self._rules:
            self._deny("poll32", resource, offset, Denial.NOT_IN_ALLOWLIST)
        timestamp = target._tick()
        if (resource, offset) in target._faults:
            target._append(
                {
                    "operation": "poll32",
                    "session": self._session,
                    "resource": resource,
                    "offset": offset,
                    "status": "backend_fault",
                    "timestamp_ns": timestamp,
                }
            )
            raise OperationDenied(Denial.BACKEND_FAULT)
        current = target._values.get((resource, offset), 0)
        if (current & mask) == (expected & mask):
            seq = target._append(
                {
                    "operation": "poll32",
                    "session": self._session,
                    "resource": resource,
                    "offset": offset,
                    "value": current,
                    "timestamp_ns": timestamp,
                }
            )
            return PollOutcome(
                value=current,
                audit_seq=seq,
                timestamp_ns=timestamp,
            )
        target._append(
            {
                "operation": "poll32",
                "session": self._session,
                "resource": resource,
                "offset": offset,
                "decision": "denied",
                "denial": Denial.TIMEOUT.value,
                "status": "rejected",
                "timestamp_ns": timestamp,
            }
        )
        raise OperationDenied(Denial.TIMEOUT)

    async def gpio_read(self, resource: int) -> GpioReadOutcome:
        """See `ProxySession.gpio_read`."""
        target = self._target
        if all(info.id != resource for info in target._description.resources):
            self._deny("gpio_read", resource, 0, Denial.UNKNOWN_RESOURCE)
        if (
            (resource, 0, AccessClass.PROTOCOL) not in self._rules
            and (resource, 0, AccessClass.READ_ONCE) not in self._rules
            and (resource, 0, AccessClass.SEQUENCE) not in self._rules
        ):
            self._deny("gpio_read", resource, 0, Denial.NOT_IN_ALLOWLIST)
        timestamp = target._tick()
        val = target._gpio_values.get(resource, False)
        seq = target._append(
            {
                "operation": "gpio_read",
                "session": self._session,
                "resource": resource,
                "offset": 0,
                "value": int(val),
                "timestamp_ns": timestamp,
            }
        )
        return GpioReadOutcome(value=val, audit_seq=seq, timestamp_ns=timestamp)

    async def gpio_write(self, resource: int, value: bool) -> GpioWriteOutcome:
        """See `ProxySession.gpio_write`."""
        target = self._target
        if self._mode != SessionMode.MUTATING:
            self._deny("gpio_write", resource, 0, Denial.READ_ONLY_SESSION)
        if all(info.id != resource for info in target._description.resources):
            self._deny("gpio_write", resource, 0, Denial.UNKNOWN_RESOURCE)
        if (
            (resource, 0, AccessClass.PROTOCOL) not in self._rules
            and (resource, 0, AccessClass.WRITE) not in self._rules
            and (resource, 0, AccessClass.SEQUENCE) not in self._rules
        ):
            self._deny("gpio_write", resource, 0, Denial.NOT_IN_ALLOWLIST)
        timestamp = target._tick()
        target._gpio_values[resource] = value
        target._gpio_writes.append((resource, value))
        seq = target._append(
            {
                "operation": "gpio_write",
                "session": self._session,
                "resource": resource,
                "offset": 0,
                "value": int(value),
                "timestamp_ns": timestamp,
            }
        )
        return GpioWriteOutcome(audit_seq=seq, timestamp_ns=timestamp)

    async def i2c_transfer(
        self, resource: int, write_data: bytes, read_length: int = 0
    ) -> I2cTransferOutcome:
        """See `ProxySession.i2c_transfer`."""
        target = self._target
        if len(write_data) > 0 and self._mode != SessionMode.MUTATING:
            self._deny("i2c_transfer", resource, 0, Denial.READ_ONLY_SESSION)
        if all(info.id != resource for info in target._description.resources):
            self._deny("i2c_transfer", resource, 0, Denial.UNKNOWN_RESOURCE)
        if (
            (resource, 0, AccessClass.PROTOCOL) not in self._rules
            and (resource, 0, AccessClass.SEQUENCE) not in self._rules
            and (resource, 0, AccessClass.READ_ONCE) not in self._rules
            and (resource, 0, AccessClass.WRITE) not in self._rules
        ):
            self._deny("i2c_transfer", resource, 0, Denial.NOT_IN_ALLOWLIST)
        timestamp = target._tick()
        target._i2c_transfers.append((resource, write_data, read_length))
        data = target._i2c_responses.get(resource, b"")
        if read_length > 0 and len(data) > read_length:
            data = data[:read_length]
        seq = target._append(
            {
                "operation": "i2c_transfer",
                "session": self._session,
                "resource": resource,
                "offset": 0,
                "value": len(data),
                "timestamp_ns": timestamp,
            }
        )
        return I2cTransferOutcome(
            read_data=data, audit_seq=seq, timestamp_ns=timestamp
        )

    async def spi_transmit(
        self, resource: int, tx_data: bytes
    ) -> SpiTransmitOutcome:
        """See `ProxySession.spi_transmit`."""
        target = self._target
        if self._mode != SessionMode.MUTATING:
            self._deny("spi_transmit", resource, 0, Denial.READ_ONLY_SESSION)
        if all(info.id != resource for info in target._description.resources):
            self._deny("spi_transmit", resource, 0, Denial.UNKNOWN_RESOURCE)
        if (
            (resource, 0, AccessClass.PROTOCOL) not in self._rules
            and (resource, 0, AccessClass.SEQUENCE) not in self._rules
            and (resource, 0, AccessClass.WRITE) not in self._rules
        ):
            self._deny("spi_transmit", resource, 0, Denial.NOT_IN_ALLOWLIST)
        timestamp = target._tick()
        target._spi_transmits.append((resource, tx_data))
        data = target._spi_responses.get(resource, b"")
        seq = target._append(
            {
                "operation": "spi_transmit",
                "session": self._session,
                "resource": resource,
                "offset": 0,
                "value": len(data),
                "timestamp_ns": timestamp,
            }
        )
        return SpiTransmitOutcome(
            rx_data=data, audit_seq=seq, timestamp_ns=timestamp
        )

    async def execute_sequence(
        self, items: Sequence[SequenceItem]
    ) -> SequenceOutcome:
        target = self._target
        if len(items) > 64:
            self._deny("sequence", 0, 0, Denial.LIMIT_EXCEEDED)
        # Whole-sequence prevalidation before first hardware access
        for index, item in enumerate(items):
            if item.kind in ("mmio_write32", "write32"):
                if self._mode != SessionMode.MUTATING:
                    target._append(
                        {
                            "operation": "sequence",
                            "session": self._session,
                            "resource": item.resource,
                            "offset": item.offset,
                            "decision": "denied",
                            "denial": Denial.READ_ONLY_SESSION.value,
                            "status": "rejected",
                            "item_index": index,
                            "timestamp_ns": target._tick(),
                        }
                    )
                    raise OperationDenied(Denial.READ_ONLY_SESSION)
                if (
                    item.resource,
                    item.offset,
                    AccessClass.WRITE,
                ) not in self._rules:
                    target._append(
                        {
                            "operation": "sequence",
                            "session": self._session,
                            "resource": item.resource,
                            "offset": item.offset,
                            "decision": "denied",
                            "denial": Denial.NOT_IN_ALLOWLIST.value,
                            "status": "rejected",
                            "item_index": index,
                            "timestamp_ns": target._tick(),
                        }
                    )
                    raise OperationDenied(Denial.NOT_IN_ALLOWLIST)
            elif item.kind in ("mmio_read32", "read32"):
                if (
                    item.resource,
                    item.offset,
                    AccessClass.SEQUENCE,
                ) not in self._rules:
                    target._append(
                        {
                            "operation": "sequence",
                            "session": self._session,
                            "resource": item.resource,
                            "offset": item.offset,
                            "decision": "denied",
                            "denial": Denial.NOT_IN_ALLOWLIST.value,
                            "status": "rejected",
                            "item_index": index,
                            "timestamp_ns": target._tick(),
                        }
                    )
                    raise OperationDenied(Denial.NOT_IN_ALLOWLIST)
            elif item.kind in ("mmio_poll32", "poll32"):
                if (
                    item.resource,
                    item.offset,
                    AccessClass.POLL,
                ) not in self._rules:
                    target._append(
                        {
                            "operation": "sequence",
                            "session": self._session,
                            "resource": item.resource,
                            "offset": item.offset,
                            "decision": "denied",
                            "denial": Denial.NOT_IN_ALLOWLIST.value,
                            "status": "rejected",
                            "item_index": index,
                            "timestamp_ns": target._tick(),
                        }
                    )
                    raise OperationDenied(Denial.NOT_IN_ALLOWLIST)
            elif item.kind == "delay_ns":
                if item.delay_ns < 0 or item.delay_ns > 1_000_000_000:
                    target._append(
                        {
                            "operation": "sequence",
                            "session": self._session,
                            "decision": "denied",
                            "denial": Denial.LIMIT_EXCEEDED.value,
                            "status": "rejected",
                            "item_index": index,
                            "timestamp_ns": target._tick(),
                        }
                    )
                    raise OperationDenied(Denial.LIMIT_EXCEEDED)
            elif item.kind == "gpio_read":
                if (
                    (item.resource, 0, AccessClass.PROTOCOL) not in self._rules
                    and (item.resource, 0, AccessClass.SEQUENCE)
                    not in self._rules
                    and (item.resource, 0, AccessClass.READ_ONCE)
                    not in self._rules
                ):
                    target._append(
                        {
                            "operation": "sequence",
                            "session": self._session,
                            "resource": item.resource,
                            "offset": 0,
                            "decision": "denied",
                            "denial": Denial.NOT_IN_ALLOWLIST.value,
                            "status": "rejected",
                            "item_index": index,
                            "timestamp_ns": target._tick(),
                        }
                    )
                    raise OperationDenied(Denial.NOT_IN_ALLOWLIST)
            elif item.kind == "gpio_write":
                if self._mode != SessionMode.MUTATING:
                    target._append(
                        {
                            "operation": "sequence",
                            "session": self._session,
                            "resource": item.resource,
                            "offset": 0,
                            "decision": "denied",
                            "denial": Denial.READ_ONLY_SESSION.value,
                            "status": "rejected",
                            "item_index": index,
                            "timestamp_ns": target._tick(),
                        }
                    )
                    raise OperationDenied(Denial.READ_ONLY_SESSION)
                if (
                    (item.resource, 0, AccessClass.PROTOCOL) not in self._rules
                    and (item.resource, 0, AccessClass.SEQUENCE)
                    not in self._rules
                    and (item.resource, 0, AccessClass.WRITE) not in self._rules
                ):
                    target._append(
                        {
                            "operation": "sequence",
                            "session": self._session,
                            "resource": item.resource,
                            "offset": 0,
                            "decision": "denied",
                            "denial": Denial.NOT_IN_ALLOWLIST.value,
                            "status": "rejected",
                            "item_index": index,
                            "timestamp_ns": target._tick(),
                        }
                    )
                    raise OperationDenied(Denial.NOT_IN_ALLOWLIST)
            elif item.kind == "i2c_transfer":
                if (
                    len(item.write_data) > 0
                    and self._mode != SessionMode.MUTATING
                ):
                    target._append(
                        {
                            "operation": "sequence",
                            "session": self._session,
                            "resource": item.resource,
                            "offset": 0,
                            "decision": "denied",
                            "denial": Denial.READ_ONLY_SESSION.value,
                            "status": "rejected",
                            "item_index": index,
                            "timestamp_ns": target._tick(),
                        }
                    )
                    raise OperationDenied(Denial.READ_ONLY_SESSION)
                if (
                    (item.resource, 0, AccessClass.PROTOCOL) not in self._rules
                    and (item.resource, 0, AccessClass.SEQUENCE)
                    not in self._rules
                    and (item.resource, 0, AccessClass.READ_ONCE)
                    not in self._rules
                    and (item.resource, 0, AccessClass.WRITE) not in self._rules
                ):
                    target._append(
                        {
                            "operation": "sequence",
                            "session": self._session,
                            "resource": item.resource,
                            "offset": 0,
                            "decision": "denied",
                            "denial": Denial.NOT_IN_ALLOWLIST.value,
                            "status": "rejected",
                            "item_index": index,
                            "timestamp_ns": target._tick(),
                        }
                    )
                    raise OperationDenied(Denial.NOT_IN_ALLOWLIST)
            elif item.kind == "spi_transmit":
                if self._mode != SessionMode.MUTATING:
                    target._append(
                        {
                            "operation": "sequence",
                            "session": self._session,
                            "resource": item.resource,
                            "offset": 0,
                            "decision": "denied",
                            "denial": Denial.READ_ONLY_SESSION.value,
                            "status": "rejected",
                            "item_index": index,
                            "timestamp_ns": target._tick(),
                        }
                    )
                    raise OperationDenied(Denial.READ_ONLY_SESSION)
                if (
                    (item.resource, 0, AccessClass.PROTOCOL) not in self._rules
                    and (item.resource, 0, AccessClass.SEQUENCE)
                    not in self._rules
                    and (item.resource, 0, AccessClass.WRITE) not in self._rules
                ):
                    target._append(
                        {
                            "operation": "sequence",
                            "session": self._session,
                            "resource": item.resource,
                            "offset": 0,
                            "decision": "denied",
                            "denial": Denial.NOT_IN_ALLOWLIST.value,
                            "status": "rejected",
                            "item_index": index,
                            "timestamp_ns": target._tick(),
                        }
                    )
                    raise OperationDenied(Denial.NOT_IN_ALLOWLIST)
        results: list[SequenceItemOutcome] = []
        complete = True
        for index, item in enumerate(items):
            timestamp = target._tick()
            if item.kind in ("mmio_read32", "read32"):
                if (item.resource, item.offset) in target._faults:
                    seq = target._append(
                        {
                            "operation": "sequence_read32",
                            "session": self._session,
                            "resource": item.resource,
                            "offset": item.offset,
                            "status": "backend_fault",
                            "item_index": index,
                            "timestamp_ns": timestamp,
                        }
                    )
                    results.append(
                        SequenceItemOutcome(
                            index=index,
                            ok=False,
                            kind="read32",
                            audit_seq=seq,
                            timestamp_ns=timestamp,
                            error=Denial.BACKEND_FAULT,
                        )
                    )
                    complete = False
                    break
                val = target._values.get((item.resource, item.offset), 0)
                seq = target._append(
                    {
                        "operation": "sequence_read32",
                        "session": self._session,
                        "resource": item.resource,
                        "offset": item.offset,
                        "value": val,
                        "item_index": index,
                        "timestamp_ns": timestamp,
                    }
                )
                results.append(
                    SequenceItemOutcome(
                        index=index,
                        ok=True,
                        kind="read32",
                        value=val,
                        audit_seq=seq,
                        timestamp_ns=timestamp,
                    )
                )
            elif item.kind in ("mmio_write32", "write32"):
                curr = target._values.get((item.resource, item.offset), 0)
                if item.precondition is not None:
                    exp, pmask = item.precondition
                    if (curr & pmask) != (exp & pmask):
                        seq = target._append(
                            {
                                "operation": "sequence_write32",
                                "session": self._session,
                                "resource": item.resource,
                                "offset": item.offset,
                                "decision": "denied",
                                "denial": Denial.PRECONDITION_FAILED.value,
                                "status": "rejected",
                                "item_index": index,
                                "timestamp_ns": timestamp,
                            }
                        )
                        results.append(
                            SequenceItemOutcome(
                                index=index,
                                ok=False,
                                kind="write32",
                                audit_seq=seq,
                                timestamp_ns=timestamp,
                                error=Denial.PRECONDITION_FAILED,
                            )
                        )
                        complete = False
                        break
                if (item.resource, item.offset) in target._faults:
                    seq = target._append(
                        {
                            "operation": "sequence_write32",
                            "session": self._session,
                            "resource": item.resource,
                            "offset": item.offset,
                            "status": "backend_fault",
                            "item_index": index,
                            "timestamp_ns": timestamp,
                        }
                    )
                    results.append(
                        SequenceItemOutcome(
                            index=index,
                            ok=False,
                            kind="write32",
                            audit_seq=seq,
                            timestamp_ns=timestamp,
                            error=Denial.BACKEND_FAULT,
                        )
                    )
                    complete = False
                    break
                new_val = (curr & ~item.write_mask) | (
                    item.value & item.write_mask
                )
                target._values[(item.resource, item.offset)] = new_val
                rb = new_val if item.readback else None
                seq = target._append(
                    {
                        "operation": "sequence_write32",
                        "session": self._session,
                        "resource": item.resource,
                        "offset": item.offset,
                        "value": new_val,
                        "item_index": index,
                        "timestamp_ns": timestamp,
                    }
                )
                results.append(
                    SequenceItemOutcome(
                        index=index,
                        ok=True,
                        kind="write32",
                        value=new_val,
                        readback_value=rb,
                        audit_seq=seq,
                        timestamp_ns=timestamp,
                    )
                )
            elif item.kind in ("mmio_poll32", "poll32"):
                curr = target._values.get((item.resource, item.offset), 0)
                if (curr & item.mask) == (item.expected & item.mask):
                    seq = target._append(
                        {
                            "operation": "sequence_poll32",
                            "session": self._session,
                            "resource": item.resource,
                            "offset": item.offset,
                            "value": curr,
                            "item_index": index,
                            "timestamp_ns": timestamp,
                        }
                    )
                    results.append(
                        SequenceItemOutcome(
                            index=index,
                            ok=True,
                            kind="poll32",
                            value=curr,
                            audit_seq=seq,
                            timestamp_ns=timestamp,
                        )
                    )
                else:
                    seq = target._append(
                        {
                            "operation": "sequence_poll32",
                            "session": self._session,
                            "resource": item.resource,
                            "offset": item.offset,
                            "decision": "denied",
                            "denial": Denial.TIMEOUT.value,
                            "status": "rejected",
                            "item_index": index,
                            "timestamp_ns": timestamp,
                        }
                    )
                    results.append(
                        SequenceItemOutcome(
                            index=index,
                            ok=False,
                            kind="poll32",
                            audit_seq=seq,
                            timestamp_ns=timestamp,
                            error=Denial.TIMEOUT,
                        )
                    )
                    complete = False
                    break
            elif item.kind == "delay_ns":
                target._now_ns += item.delay_ns
                results.append(
                    SequenceItemOutcome(
                        index=index,
                        ok=True,
                        kind="delay_ns",
                        timestamp_ns=timestamp,
                    )
                )
            elif item.kind == "barrier":
                results.append(
                    SequenceItemOutcome(
                        index=index,
                        ok=True,
                        kind="barrier",
                        timestamp_ns=timestamp,
                    )
                )
            elif item.kind == "gpio_read":
                val = target._gpio_values.get(item.resource, False)
                seq = target._append(
                    {
                        "operation": "sequence_gpio_read",
                        "session": self._session,
                        "resource": item.resource,
                        "offset": 0,
                        "value": int(val),
                        "item_index": index,
                        "timestamp_ns": timestamp,
                    }
                )
                results.append(
                    SequenceItemOutcome(
                        index=index,
                        ok=True,
                        kind="gpio_read",
                        value=int(val),
                        audit_seq=seq,
                        timestamp_ns=timestamp,
                    )
                )
            elif item.kind == "gpio_write":
                val = bool(item.value)
                target._gpio_values[item.resource] = val
                target._gpio_writes.append((item.resource, val))
                seq = target._append(
                    {
                        "operation": "sequence_gpio_write",
                        "session": self._session,
                        "resource": item.resource,
                        "offset": 0,
                        "value": int(val),
                        "item_index": index,
                        "timestamp_ns": timestamp,
                    }
                )
                results.append(
                    SequenceItemOutcome(
                        index=index,
                        ok=True,
                        kind="gpio_write",
                        value=int(val),
                        audit_seq=seq,
                        timestamp_ns=timestamp,
                    )
                )
            elif item.kind == "i2c_transfer":
                target._i2c_transfers.append(
                    (item.resource, item.write_data, item.read_length)
                )
                data = target._i2c_responses.get(item.resource, b"")
                if item.read_length > 0 and len(data) > item.read_length:
                    data = data[: item.read_length]
                seq = target._append(
                    {
                        "operation": "sequence_i2c_transfer",
                        "session": self._session,
                        "resource": item.resource,
                        "offset": 0,
                        "value": len(data),
                        "item_index": index,
                        "timestamp_ns": timestamp,
                    }
                )
                results.append(
                    SequenceItemOutcome(
                        index=index,
                        ok=True,
                        kind="i2c_transfer",
                        data=data,
                        audit_seq=seq,
                        timestamp_ns=timestamp,
                    )
                )
            elif item.kind == "spi_transmit":
                target._spi_transmits.append((item.resource, item.tx_data))
                data = target._spi_responses.get(item.resource, b"")
                seq = target._append(
                    {
                        "operation": "sequence_spi_transmit",
                        "session": self._session,
                        "resource": item.resource,
                        "offset": 0,
                        "value": len(data),
                        "item_index": index,
                        "timestamp_ns": timestamp,
                    }
                )
                results.append(
                    SequenceItemOutcome(
                        index=index,
                        ok=True,
                        kind="spi_transmit",
                        data=data,
                        audit_seq=seq,
                        timestamp_ns=timestamp,
                    )
                )
        return SequenceOutcome(results=tuple(results), complete=complete)

    async def close(self) -> None:
        """See `ProxySession.close`."""
        target = self._target
        if (
            self._mode == SessionMode.MUTATING
            and target.active_mutating_session == self._session
        ):
            target.active_mutating_session = None
        target._append(
            {
                "operation": "session_closed",
                "session": self._session,
                "timestamp_ns": target._tick(),
            }
        )


@dataclasses.dataclass(frozen=True)
class DirectDescription:
    """Description of a direct published-protocol target."""

    node_id: str
    protocol_name: str
    boot_id: str
    bound_driver_url: str | None = None


@dataclasses.dataclass(frozen=True)
class FidlCallOutcome:
    """Outcome of a single published FIDL method call."""

    method: str
    response: Mapping[str, object]
    timestamp_ns: int = 0


@runtime_checkable
class DirectSession(Protocol):
    """An open session interacting directly with a published FIDL protocol."""

    async def call_fidl(
        self, method: str, args: Mapping[str, object] | None = None
    ) -> FidlCallOutcome:
        """Invokes a method on the published protocol."""
        ...

    async def close(self) -> None:
        """Closes the direct session."""
        ...


@runtime_checkable
class DirectTransport(Protocol):
    """Transport connecting to a published protocol without the proxy driver."""

    is_direct: bool = True

    async def describe(self) -> DirectDescription:
        """Describes the published target."""
        ...

    async def open_direct_session(
        self, context: SessionContext
    ) -> DirectSession:
        """Opens a direct session against the published protocol."""
        ...


class FakeDirectTarget:
    """In-memory direct target for testing published-protocol interactions."""

    is_direct = True

    def __init__(
        self,
        node_id: str = "example-device",
        protocol_name: str = "fuchsia.hardware.example/Device",
        boot_id: str = "boot-1",
        bound_driver_url: str | None = "fuchsia-boot:///example#meta/driver.cm",
    ) -> None:
        self.node_id = node_id
        self.protocol_name = protocol_name
        self.boot_id = boot_id
        self.bound_driver_url = bound_driver_url
        self.handlers: dict[
            str, Callable[[Mapping[str, object]], Mapping[str, object]]
        ] = {}
        self.sessions_opened = 0
        self.fail_open = False
        self.calls: list[tuple[str, Mapping[str, object]]] = []
        self._clock = 1_000_000

    def register_method(
        self,
        method: str,
        handler: Callable[[Mapping[str, object]], Mapping[str, object]],
    ) -> None:
        self.handlers[method] = handler

    async def describe(self) -> DirectDescription:
        return DirectDescription(
            node_id=self.node_id,
            protocol_name=self.protocol_name,
            boot_id=self.boot_id,
            bound_driver_url=self.bound_driver_url,
        )

    async def open_direct_session(
        self, context: SessionContext
    ) -> DirectSession:
        if self.fail_open:
            raise TransportError("failed to connect to published protocol")
        self.sessions_opened += 1
        return _FakeDirectSession(self)


class _FakeDirectSession:
    def __init__(self, target: FakeDirectTarget) -> None:
        self._target = target

    async def call_fidl(
        self, method: str, args: Mapping[str, object] | None = None
    ) -> FidlCallOutcome:
        args_clean = dict(args or {})
        self._target.calls.append((method, args_clean))
        handler = self._target.handlers.get(method)
        if handler is None:
            raise OperationDenied(Denial.UNKNOWN_RESOURCE)
        res = handler(args_clean)
        self._target._clock += 1_000
        return FidlCallOutcome(
            method=method,
            response=res,
            timestamp_ns=self._target._clock,
        )

    async def close(self) -> None:
        pass
