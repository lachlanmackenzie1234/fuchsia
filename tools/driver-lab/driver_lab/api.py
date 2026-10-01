# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

"""Public phase 1 host API: plan-driven runs composing consent, the
transport, and evidence.

The workflow is prepare (validate the plan, resolve consent -- no target
access), execute (open a session with the exact plan-derived allowlist,
run bounded operations, drain audit), and finalize (hash evidence,
manifest last). Missing consent fails closed, and no denial or failure
skips evidence finalization.
"""

from __future__ import annotations

import asyncio
import dataclasses
from collections.abc import Callable, Mapping, Sequence
from pathlib import Path
from typing import Any, Literal

from driver_lab.consent import (
    READ_WARNING,
    WRITE_WARNING,
    ConsentDecision,
    ConsentPrompt,
    grant_from_decision,
)
from driver_lab.discovery import (
    DiscoveryError,
    NodeDescription,
    NodeDiscovery,
    NodeSummary,
)
from driver_lab.evidence import EvidenceRecorder
from driver_lab.models import AccessClass, AccessRequest
from driver_lab.permissions import (
    GrantStoreError,
    Outcome,
    Resolution,
    add_grant,
    load_grants,
    resolve,
)
from driver_lab.plans import is_mutating_plan, plan_digest, validate_plan
from driver_lab.session import (
    AccessRequirements,
    HardwareSession,
    SessionCapabilities,
)
from driver_lab.transport import (
    STALE_REJECTIONS,
    AllowRule,
    DirectTransport,
    Expectations,
    OpenSessionRejected,
    OperationDenied,
    ProxyDescription,
    ProxyTransport,
    ResourceInfo,
    SequenceItem,
    SessionContext,
    SessionMode,
    SnapshotItem,
    TransportError,
)

EXIT_SUCCESS = 0
EXIT_PERMISSION = 2
EXIT_STALE = 3
EXIT_OPERATION = 4
EXIT_TRANSPORT = 5
EXIT_EVIDENCE = 7
EXIT_ACTIVATION = 8
EXIT_UNSUPPORTED = 10

# What a phase 1 proxy-mode run guarantees. Requirements a plan declares
# are checked against this before any target connection, so an
# unsupported guarantee never silently downgrades.
PROXY_CAPABILITIES = {
    "mode": "proxy",
    "target_policy": True,
    "target_audit": True,
    "target_local_timing": False,
}

# Direct mode connects to a published protocol without proxy mediation.
# It makes no claim to private MMIO, target policy, or target audit.
DIRECT_CAPABILITIES = {
    "mode": "direct",
    "target_policy": False,
    "target_audit": False,
    "target_local_timing": False,
}

_DEFERRED_ARTIFACTS = (
    "target.description.json",
    "permission-resolution.json",
    "operations.jsonl",
    "target-audit.jsonl",
)

# Spec-defined artifacts a phase 1 read-only run can never produce.
# Marked not applicable rather than silently omitted: takeover
# restoration and proxy activation are phase 2; mutation approval,
# node discovery snapshots, serial capture, and the interpretation
# layer have not landed yet.
_NOT_APPLICABLE_ARTIFACTS = (
    "approval.json",
    "node.before.json",
    "node.after.json",
    "proxy-access.jsonl",
    "restoration.json",
    "serial.log",
    "host-events.jsonl",
    "interpretation.json",
)


class DriverLabError(Exception):
    """A host-side configuration or usage error."""


@dataclasses.dataclass(frozen=True)
class ReadRecord:
    """One completed 32-bit read from a plan run."""

    operation_index: int
    resource: str
    offset: int
    value: int
    audit_seq: int
    timestamp_ns: int


@dataclasses.dataclass(frozen=True)
class WriteRecord:
    """One completed 32-bit write from a plan run."""

    operation_index: int
    resource: str
    offset: int
    value: int
    write_mask: int
    readback_value: int | None
    audit_seq: int
    timestamp_ns: int


@dataclasses.dataclass(frozen=True)
class PollRecord:
    """One completed 32-bit poll from a plan run."""

    operation_index: int
    resource: str
    offset: int
    expected: int
    mask: int
    value: int
    audit_seq: int
    timestamp_ns: int


@dataclasses.dataclass(frozen=True)
class SequenceItemRecord:
    """Outcome of one item executed in a sequence."""

    index: int
    ok: bool
    kind: str
    value: int | None = None
    readback_value: int | None = None
    audit_seq: int = 0
    timestamp_ns: int = 0
    error: str | None = None


@dataclasses.dataclass(frozen=True)
class SequenceRecord:
    """One executed sequence from a plan run."""

    operation_index: int
    results: tuple[SequenceItemRecord, ...]
    complete: bool


@dataclasses.dataclass(frozen=True)
class RunResult:
    """The outcome of one plan run. Evidence is always finalized."""

    exit_category: int
    evidence_dir: Path
    plan_digest: str
    reads: tuple[ReadRecord, ...] = ()
    writes: tuple[WriteRecord, ...] = ()
    polls: tuple[PollRecord, ...] = ()
    sequences: tuple[SequenceRecord, ...] = ()
    calls: tuple[Mapping[str, Any], ...] = ()
    failure: str | None = None

    @property
    def ok(self) -> bool:
        """Whether the run succeeded completely."""
        return self.exit_category == EXIT_SUCCESS


@dataclasses.dataclass(frozen=True)
class _ReadOp:
    index: int
    resource: ResourceInfo
    offset: int


@dataclasses.dataclass(frozen=True)
class _SnapshotOp:
    index: int
    items: tuple[tuple[ResourceInfo, int], ...]


@dataclasses.dataclass(frozen=True)
class _WriteOp:
    index: int
    resource: ResourceInfo
    offset: int
    value: int
    write_mask: int
    precondition: tuple[int, int] | None
    readback: bool


@dataclasses.dataclass(frozen=True)
class _PollOp:
    index: int
    resource: ResourceInfo
    offset: int
    expected: int
    mask: int
    interval_ns: int
    timeout_ns: int


@dataclasses.dataclass(frozen=True)
class _SequenceOp:
    index: int
    items: tuple[SequenceItem, ...]
    has_mutation: bool


class _StaleResource(Exception):
    pass


class DriverLab:
    """Facade over one target for plan-driven runs."""

    def __init__(
        self,
        transport: ProxyTransport | DirectTransport,
        *,
        grants_path: Path,
        evidence_root: Path,
        target_scope: str,
        node_id: str,
        consent: ConsentPrompt | None = None,
        discovery: NodeDiscovery | None = None,
    ) -> None:
        self._transport = transport
        self._grants_path = grants_path
        self._evidence_root = evidence_root
        self._target_scope = target_scope
        self._node_id = node_id
        self._consent = consent
        self._discovery = discovery

    async def list_nodes(
        self,
        node_filter: list[str] | None = None,
        exact_match: bool = False,
    ) -> list[NodeSummary]:
        """Discovers nodes on the target control plane."""
        if self._discovery is None:
            raise DriverLabError(
                "node discovery is not configured for this DriverLab instance"
            )
        return await self._discovery.list_nodes(node_filter, exact_match)

    async def describe_node(self, node_id: str) -> NodeDescription | None:
        """Describes a node by moniker."""
        if self._discovery is None:
            raise DriverLabError(
                "node discovery is not configured for this DriverLab instance"
            )
        return await self._discovery.describe_node(node_id)

    @classmethod
    async def connect(
        cls,
        target: str = "default",
        *,
        transport: str = "auto",
        timeout_s: float = 10.0,
        grants_path: Path | None = None,
        evidence_root: Path | None = None,
        target_scope: str | None = None,
        node_id: str | None = None,
        consent: ConsentPrompt | None = None,
        discovery: NodeDiscovery | None = None,
        proxy_transport: ProxyTransport | None = None,
        direct_transport: DirectTransport | None = None,
    ) -> "DriverLab":
        """Connects to a target device, establishing transports according to mode."""
        active_transport: ProxyTransport | DirectTransport
        if proxy_transport is not None:
            active_transport = proxy_transport
        elif direct_transport is not None:
            active_transport = direct_transport
        elif transport in ("proxy", "auto"):
            from driver_lab.fidl_transport import connect_transport

            active_transport = connect_transport(
                node_id or target, target=target
            )
        else:
            raise DriverLabError(
                "Direct transport must be explicitly provided via direct_transport"
            )

        return cls(
            transport=active_transport,
            grants_path=grants_path or Path("grants.toml"),
            evidence_root=evidence_root or Path("evidence"),
            target_scope=target_scope or target,
            node_id=node_id or target,
            consent=consent,
            discovery=discovery,
        )

    async def attach(
        self,
        node_id: str,
        *,
        mode: Literal["auto", "direct", "proxy"] = "auto",
        requirements: AccessRequirements | None = None,
        session_mode: SessionMode = SessionMode.READ_ONLY,
        allowlist: Sequence[AllowRule] | None = None,
        expectations: Expectations | Mapping[str, Any] | None = None,
        context: SessionContext | None = None,
        consent: ConsentPrompt | None = None,
    ) -> HardwareSession:
        """Attaches to a node, returning a driver-shaped HardwareSession (Spec Sections 6.1, 7.3, 9)."""
        if mode not in ("auto", "direct", "proxy"):
            raise ValueError(
                f"Invalid mode '{mode}': must be 'auto', 'direct', or 'proxy'"
            )

        if mode == "auto":
            if requirements and (
                requirements.needs_mmio
                or requirements.needs_sequence
                or requirements.needs_target_timing
                or requirements.needs_target_policy
                or requirements.needs_target_audit
                or requirements.is_mutating
            ):
                selected_mode = "proxy"
            elif isinstance(self._transport, DirectTransport) or (
                requirements and requirements.protocol
            ):
                selected_mode = "direct"
            else:
                selected_mode = "proxy"
        elif mode == "direct":
            if requirements and (
                requirements.needs_mmio
                or requirements.needs_sequence
                or requirements.needs_target_timing
                or requirements.needs_target_policy
                or requirements.needs_target_audit
                or requirements.is_mutating
            ):
                raise DriverLabError(
                    "Direct mode cannot satisfy requested requirements (requires proxy mode)"
                )
            selected_mode = "direct"
        else:
            selected_mode = "proxy"

        if selected_mode == "direct":
            if not isinstance(self._transport, DirectTransport):
                raise DriverLabError(
                    "Direct transport is not configured for this DriverLab instance"
                )
            direct_desc = await self._transport.describe()
            direct_ctx = context or SessionContext(
                run_id=f"direct-{node_id}",
                case_id="interactive",
                plan_digest="direct-session",
                host_tool_version="0.1.0",
            )
            direct_session = await self._transport.open_direct_session(
                direct_ctx
            )
            caps = SessionCapabilities(
                mode="direct",
                target_policy=False,
                target_audit=False,
                target_local_timing=False,
                fault_isolation="driver_host",
                production_driver_active=bool(direct_desc.bound_driver_url),
                restoration_required=False,
                resources=(),
                protocols=(direct_desc.protocol_name,),
            )
            return HardwareSession(
                capabilities=caps,
                direct_session=direct_session,
                direct_description=direct_desc,
            )

        # Proxy mode
        if not isinstance(self._transport, ProxyTransport):
            raise DriverLabError(
                "Proxy transport is not configured for this DriverLab instance"
            )
        proxy_desc = await self._transport.describe()

        if session_mode == SessionMode.MUTATING:
            active_consent = consent or self._consent
            if active_consent is None:
                raise DriverLabError(
                    "Mutating session requires operator consent, but no consent prompt is configured"
                )
            req = AccessRequest(
                target_scope=self._target_scope,
                node_id=node_id,
                resource_digest=proxy_desc.resource_digest,
                resource="*",
                offset=0,
                width=4,
                access=AccessClass.WRITE,
            )
            decision = await active_consent.request_consent(req, WRITE_WARNING)
            if decision != ConsentDecision.ALLOW_ONCE:
                raise DriverLabError(
                    f"Mutating session was not authorized by operator (decision: {decision})"
                )

        rules: list[AllowRule] = []
        if allowlist is not None:
            rules = list(allowlist)
        else:
            for res in proxy_desc.resources:
                for off in range(0, min(res.logical_size, 256), 4):
                    rules.append(
                        AllowRule(
                            resource=res.id,
                            offset=off,
                            width=4,
                            access=AccessClass.READ_ONCE,
                        )
                    )
                    rules.append(
                        AllowRule(
                            resource=res.id,
                            offset=off,
                            width=4,
                            access=AccessClass.SNAPSHOT,
                        )
                    )
                    rules.append(
                        AllowRule(
                            resource=res.id,
                            offset=off,
                            width=4,
                            access=AccessClass.POLL,
                        )
                    )
                    rules.append(
                        AllowRule(
                            resource=res.id,
                            offset=off,
                            width=4,
                            access=AccessClass.SEQUENCE,
                        )
                    )
                    if session_mode == SessionMode.MUTATING:
                        rules.append(
                            AllowRule(
                                resource=res.id,
                                offset=off,
                                width=4,
                                access=AccessClass.WRITE,
                            )
                        )
                    if len(rules) >= 250:
                        break
                if len(rules) >= 250:
                    break

        proxy_ctx = context or SessionContext(
            run_id=f"proxy-{node_id}",
            case_id="interactive",
            plan_digest="proxy-session",
            host_tool_version="0.1.0",
        )
        exp: Expectations
        if expectations is not None:
            if isinstance(expectations, Expectations):
                exp = expectations
            else:
                exp = Expectations(
                    boot_id=str(expectations["boot_id"]),
                    proxy_generation=int(expectations["proxy_generation"]),
                    resource_digest=str(expectations["resource_digest"]),
                    policy_digest=str(expectations["policy_digest"]),
                )
        else:
            exp = Expectations(
                boot_id=proxy_desc.boot_id,
                proxy_generation=proxy_desc.proxy_generation,
                resource_digest=proxy_desc.resource_digest,
                policy_digest=proxy_desc.policy_digest,
            )
        proxy_session = await self._transport.open_session(
            context=proxy_ctx,
            mode=session_mode,
            expectations=exp,
            allowlist=rules,
        )
        caps = SessionCapabilities(
            mode="proxy",
            target_policy=True,
            target_audit=True,
            target_local_timing=True,
            fault_isolation="driver_host",
            production_driver_active=False,
            restoration_required=False,
            resources=tuple(r.name for r in proxy_desc.resources),
            protocols=(),
        )

        cursor = 0

        async def _drain_audit() -> None:
            nonlocal cursor
            page = await proxy_session.read_audit(cursor=cursor, limit=64)
            cursor = page.next_cursor

        return HardwareSession(
            capabilities=caps,
            proxy_session=proxy_session,
            proxy_description=proxy_desc,
            audit_drainer=_drain_audit,
        )

    def _derive(
        self, canonical: Mapping[str, Any], description: ProxyDescription
    ) -> tuple[
        list[_ReadOp | _SnapshotOp | _WriteOp | _PollOp | _SequenceOp],
        list[tuple[int, AccessRequest]],
    ]:
        operations: list[
            _ReadOp | _SnapshotOp | _WriteOp | _PollOp | _SequenceOp
        ] = []
        requests: list[tuple[int, AccessRequest]] = []

        def lookup(name: str) -> ResourceInfo:
            resource = description.resource_named(name)
            if resource is None:
                raise _StaleResource(f"resource not offered by target: {name}")
            return resource

        def request(
            resource: ResourceInfo, offset: int, access: AccessClass
        ) -> AccessRequest:
            return AccessRequest(
                target_scope=self._target_scope,
                node_id=self._node_id,
                resource_digest=resource.digest,
                resource=resource.name,
                offset=offset,
                width=4,
                access=access,
            )

        for index, operation in enumerate(canonical["operations"]):
            kind = operation["kind"]
            if kind == "mmio_read32":
                resource = lookup(operation["resource"])
                offset = operation["offset"]
                operations.append(
                    _ReadOp(index=index, resource=resource, offset=offset)
                )
                requests.append(
                    (index, request(resource, offset, AccessClass.READ_ONCE))
                )
            elif kind == "mmio_snapshot32":
                items = tuple(
                    (lookup(item["resource"]), item["offset"])
                    for item in operation["items"]
                )
                operations.append(_SnapshotOp(index=index, items=items))
                requests.extend(
                    (index, request(resource, offset, AccessClass.SNAPSHOT))
                    for resource, offset in items
                )
            elif kind == "mmio_write32":
                resource = lookup(operation["resource"])
                offset = operation["offset"]
                precondition = None
                if (
                    "precondition" in operation
                    and operation["precondition"] is not None
                ):
                    precondition = (
                        operation["precondition"]["expected"],
                        operation["precondition"]["mask"],
                    )
                operations.append(
                    _WriteOp(
                        index=index,
                        resource=resource,
                        offset=offset,
                        value=operation["value"],
                        write_mask=operation.get("write_mask", 0xFFFF_FFFF),
                        precondition=precondition,
                        readback=operation.get("readback", True),
                    )
                )
                requests.append(
                    (index, request(resource, offset, AccessClass.WRITE))
                )
            elif kind == "mmio_poll32":
                resource = lookup(operation["resource"])
                offset = operation["offset"]
                operations.append(
                    _PollOp(
                        index=index,
                        resource=resource,
                        offset=offset,
                        expected=operation["expected"],
                        mask=operation.get("mask", 0xFFFF_FFFF),
                        interval_ns=operation["interval_ns"],
                        timeout_ns=operation["timeout_ns"],
                    )
                )
                requests.append(
                    (index, request(resource, offset, AccessClass.POLL))
                )
            elif kind == "sequence":
                seq_items: list[SequenceItem] = []
                has_mutation = False
                for item in operation["items"]:
                    ikind = item["kind"]
                    if ikind == "mmio_read32":
                        res = lookup(item["resource"])
                        off = item["offset"]
                        seq_items.append(
                            SequenceItem(
                                kind="read32",
                                resource=res.id,
                                offset=off,
                            )
                        )
                        requests.append(
                            (index, request(res, off, AccessClass.SEQUENCE))
                        )
                    elif ikind == "mmio_write32":
                        res = lookup(item["resource"])
                        off = item["offset"]
                        has_mutation = True
                        p = None
                        if (
                            "precondition" in item
                            and item["precondition"] is not None
                        ):
                            p = (
                                item["precondition"]["expected"],
                                item["precondition"]["mask"],
                            )
                        seq_items.append(
                            SequenceItem(
                                kind="write32",
                                resource=res.id,
                                offset=off,
                                value=item["value"],
                                write_mask=item.get("write_mask", 0xFFFF_FFFF),
                                precondition=p,
                                readback=item.get("readback", True),
                            )
                        )
                        requests.append(
                            (index, request(res, off, AccessClass.WRITE))
                        )
                    elif ikind == "mmio_poll32":
                        res = lookup(item["resource"])
                        off = item["offset"]
                        seq_items.append(
                            SequenceItem(
                                kind="poll32",
                                resource=res.id,
                                offset=off,
                                expected=item["expected"],
                                mask=item.get("mask", 0xFFFF_FFFF),
                                interval_ns=item["interval_ns"],
                                timeout_ns=item["timeout_ns"],
                            )
                        )
                        requests.append(
                            (index, request(res, off, AccessClass.POLL))
                        )
                    elif ikind == "delay_ns":
                        seq_items.append(
                            SequenceItem(
                                kind="delay_ns",
                                delay_ns=item["duration_ns"],
                            )
                        )
                    elif ikind == "barrier":
                        seq_items.append(
                            SequenceItem(
                                kind="barrier",
                                barrier=item.get("variant", "memory"),
                            )
                        )
                operations.append(
                    _SequenceOp(
                        index=index,
                        items=tuple(seq_items),
                        has_mutation=has_mutation,
                    )
                )
        return operations, requests

    async def _run_direct(
        self,
        canonical: Mapping[str, Any],
        digest: str,
        recorder: EvidenceRecorder,
        finish: Callable[[int, str | None], RunResult],
        calls: list[Mapping[str, Any]],
    ) -> RunResult:
        recorder.mark_not_applicable("permission-resolution.json")
        recorder.mark_not_applicable("target-audit.jsonl")

        context = SessionContext(
            run_id=canonical["run_id"],
            case_id=canonical["case_id"],
            plan_digest=digest,
        )
        try:
            assert isinstance(self._transport, DirectTransport)
            direct_session = await self._transport.open_direct_session(context)
        except TransportError as error:
            return finish(EXIT_TRANSPORT, f"direct connection failed: {error}")

        exit_category = EXIT_SUCCESS
        failure: str | None = None
        operation_rows: list[dict[str, object]] = []

        for index, op in enumerate(canonical["operations"]):
            method = op["method"]
            args = op.get("args", {})
            try:
                outcome = await direct_session.call_fidl(method, args)
                row = {
                    "operation": index,
                    "kind": "fidl_call",
                    "method": method,
                    "args": args,
                    "response": outcome.response,
                    "timestamp_ns": outcome.timestamp_ns,
                }
                operation_rows.append(row)
                calls.append(row)
            except OperationDenied as error:
                operation_rows.append(
                    {
                        "operation": index,
                        "kind": "fidl_call",
                        "method": method,
                        "error": error.denial.value,
                    }
                )
                exit_category = EXIT_OPERATION
                failure = error.denial.value
                break
            except TransportError as error:
                operation_rows.append(
                    {
                        "operation": index,
                        "kind": "fidl_call",
                        "method": method,
                        "error": str(error),
                    }
                )
                exit_category = EXIT_TRANSPORT
                failure = f"direct FIDL call failed: {error}"
                break
            except (KeyboardInterrupt, asyncio.CancelledError):
                exit_category = EXIT_OPERATION
                failure = "cancelled by operator"
                break

        recorder.write_jsonl("operations.jsonl", operation_rows)

        try:
            await direct_session.close()
        except TransportError as error:
            if failure is None:
                exit_category = EXIT_TRANSPORT
                failure = f"session close failed: {error}"

        return finish(exit_category, failure)

    async def run_plan(self, plan: Mapping[str, object]) -> RunResult:
        """Runs one validated plan to a finalized evidence bundle.

        Raises `PlanError` for a structurally invalid plan and
        `EvidenceError` when the evidence directory cannot be created;
        every other failure is reported through the returned
        `RunResult` with its evidence finalized.
        """
        canonical = validate_plan(plan)
        digest = plan_digest(plan)
        recorder = EvidenceRecorder(self._evidence_root, canonical["run_id"])
        recorder.write_json("plan.requested.json", dict(plan))
        recorder.write_json(
            "plan.canonical.json", {"digest": digest, "plan": canonical}
        )

        reads: list[ReadRecord] = []
        writes: list[WriteRecord] = []
        polls: list[PollRecord] = []
        sequences: list[SequenceRecord] = []
        calls: list[Mapping[str, Any]] = []

        def finish(exit_category: int, failure: str | None) -> RunResult:
            for name in _DEFERRED_ARTIFACTS + _NOT_APPLICABLE_ARTIFACTS:
                if not recorder.recorded(name):
                    recorder.mark_not_applicable(name)
            recorder.finalize(
                exit_category,
                {
                    "run_id": canonical["run_id"],
                    "case_id": canonical["case_id"],
                    "plan_digest": digest,
                    "failure": failure,
                },
            )
            return RunResult(
                exit_category=exit_category,
                evidence_dir=recorder.directory,
                plan_digest=digest,
                reads=tuple(reads),
                writes=tuple(writes),
                polls=tuple(polls),
                sequences=tuple(sequences),
                calls=tuple(calls),
                failure=failure,
            )

        # Mode and guarantee resolution happen before any target
        # connection: an unsupported mode or guarantee fails here, never
        # by silently substituting a different backend.
        access = canonical["access"]
        requested_mode = access["mode"]
        is_direct_transport = getattr(self._transport, "is_direct", False)

        if requested_mode == "direct":
            if not is_direct_transport:
                return finish(
                    EXIT_UNSUPPORTED,
                    "transport does not support direct mode; a direct-mode plan must never silently execute over the proxy",
                )
            capabilities = DIRECT_CAPABILITIES
        elif requested_mode == "proxy":
            if is_direct_transport:
                return finish(
                    EXIT_UNSUPPORTED,
                    "transport does not support proxy mode",
                )
            capabilities = PROXY_CAPABILITIES
        else:
            return finish(
                EXIT_UNSUPPORTED,
                f"access.mode {requested_mode!r} is not implemented",
            )

        recorder.write_json("access.capabilities.json", capabilities)

        for flag, capability in (
            ("requires_target_policy", "target_policy"),
            ("requires_target_audit", "target_audit"),
            ("requires_target_local_timing", "target_local_timing"),
        ):
            if access[flag] and not capabilities[capability]:
                return finish(
                    EXIT_UNSUPPORTED,
                    f"plan requires {capability}, which {requested_mode} mode does not provide",
                )

        if requested_mode == "direct":
            if any(
                op["kind"]
                in (
                    "mmio_read32",
                    "mmio_snapshot32",
                    "mmio_write32",
                    "mmio_poll32",
                    "sequence",
                )
                for op in canonical["operations"]
            ):
                return finish(
                    EXIT_UNSUPPORTED,
                    "direct mode does not provide private MMIO access",
                )
        elif requested_mode == "proxy":
            if any(op["kind"] == "fidl_call" for op in canonical["operations"]):
                return finish(
                    EXIT_UNSUPPORTED,
                    "proxy mode in phase 1 does not support fidl_call",
                )

        if "expected_unclaimed" in canonical["node"]:
            if self._discovery is None:
                # Node discovery is not configured, so the assertion cannot be
                # verified; fail closed rather than running unverified.
                return finish(
                    EXIT_UNSUPPORTED,
                    "node.expected_unclaimed cannot be verified without node discovery",
                )
            expected_unclaimed = canonical["node"]["expected_unclaimed"]
            node_id = canonical["node"]["id"]
            try:
                node_desc = await self._discovery.describe_node(node_id)
            except DiscoveryError as exc:
                return finish(
                    EXIT_ACTIVATION,
                    f"node discovery failed for {node_id}: {exc}",
                )
            if node_desc is None:
                return finish(
                    EXIT_ACTIVATION,
                    f"node {node_id} not found during discovery",
                )
            if expected_unclaimed and not node_desc.is_unclaimed:
                return finish(
                    EXIT_ACTIVATION,
                    f"node {node_id} is bound to {node_desc.bound_driver_url}; "
                    "managed takeover is phase 2",
                )
            if not expected_unclaimed and node_desc.is_unclaimed:
                return finish(
                    EXIT_ACTIVATION,
                    f"node {node_id} is unclaimed, expected bound driver",
                )

        # Prepare: describe, freeze expectations, resolve consent. No
        # hardware access happens in this phase.
        try:
            description = await self._transport.describe()
        except TransportError as error:
            return finish(EXIT_TRANSPORT, f"describe failed: {error}")
        recorder.write_json(
            "target.description.json", dataclasses.asdict(description)
        )

        expected_boot_id = canonical["target"].get("expected_boot_id")
        if (
            expected_boot_id is not None
            and expected_boot_id != description.boot_id
        ):
            return finish(EXIT_STALE, "expected_boot_id does not match target")

        expected_digest = canonical["node"].get("expected_resource_digest")
        if expected_digest is not None:
            if requested_mode == "direct":
                return finish(
                    EXIT_STALE,
                    "direct mode target has no proxy resource digest",
                )
            assert isinstance(description, ProxyDescription)
            if expected_digest != description.resource_digest:
                return finish(
                    EXIT_STALE, "expected_resource_digest does not match target"
                )

        if requested_mode == "direct":
            return await self._run_direct(
                canonical, digest, recorder, finish, calls
            )

        assert isinstance(description, ProxyDescription)
        assert isinstance(self._transport, ProxyTransport)
        try:
            operations, requests = self._derive(canonical, description)
        except _StaleResource as error:
            return finish(EXIT_STALE, str(error))

        try:
            grants = load_grants(self._grants_path)
        except GrantStoreError as error:
            return finish(EXIT_PERMISSION, f"grant store error: {error}")

        resolved: list[tuple[int, AccessRequest, Resolution]] = []
        undecided: dict[
            tuple[str, str, str, str, int, int, AccessClass], AccessRequest
        ] = {}
        persistent_denied = False
        for index, access_request in requests:
            resolution = resolve(access_request, grants)
            resolved.append((index, access_request, resolution))
            if resolution.outcome is Outcome.DENIED:
                persistent_denied = True
            elif resolution.outcome is Outcome.UNDECIDED:
                undecided.setdefault(access_request.match_key, access_request)

        # A persistent deny short-circuits prompting: the operator already
        # made a durable decision, so nobody is asked to re-litigate it.
        prompted: dict[
            tuple[str, str, str, str, int, int, AccessClass], ConsentDecision
        ] = {}
        if not persistent_denied and undecided and self._consent is not None:
            for access_request in undecided.values():
                warning = (
                    WRITE_WARNING
                    if access_request.access == AccessClass.WRITE
                    else READ_WARNING
                )
                decision = await self._consent.request_consent(
                    access_request, warning
                )
                prompted[access_request.match_key] = decision
                if decision in (
                    ConsentDecision.ALWAYS_ALLOW,
                    ConsentDecision.ALWAYS_DENY,
                ):
                    if (
                        decision == ConsentDecision.ALWAYS_ALLOW
                        and access_request.access
                        in (AccessClass.WRITE, AccessClass.SEQUENCE)
                    ):
                        return finish(
                            EXIT_PERMISSION,
                            f"persistent {access_request.access.value} grants are not supported",
                        )
                    try:
                        add_grant(
                            self._grants_path,
                            grant_from_decision(access_request, decision),
                        )
                    except (GrantStoreError, ValueError) as error:
                        return finish(
                            EXIT_PERMISSION, f"grant store error: {error}"
                        )

        resolution_rows: list[dict[str, object]] = []
        operator_denied = False
        fail_closed = False
        for index, access_request, resolution in resolved:
            grant_id: str | None = None
            if resolution.decided:
                outcome_value = resolution.outcome.value
                source = "persistent"
                grant_id = (
                    resolution.grant.grant_id if resolution.grant else None
                )
            elif access_request.match_key in prompted:
                decision = prompted[access_request.match_key]
                allowed = decision in (
                    ConsentDecision.ALLOW_ONCE,
                    ConsentDecision.ALWAYS_ALLOW,
                )
                outcome_value = (
                    Outcome.ALLOWED if allowed else Outcome.DENIED
                ).value
                operator_denied = operator_denied or not allowed
                source = decision.value
            else:
                outcome_value = Outcome.UNDECIDED.value
                source = "fail_closed"
                fail_closed = True
            resolution_rows.append(
                {
                    "operation": index,
                    "resource": access_request.resource,
                    "offset": access_request.offset,
                    "access": access_request.access.value,
                    "outcome": outcome_value,
                    "source": source,
                    "grant_id": grant_id,
                }
            )
        recorder.write_json("permission-resolution.json", resolution_rows)
        if persistent_denied:
            return finish(EXIT_PERMISSION, "denied by persistent grant")
        if operator_denied:
            return finish(EXIT_PERMISSION, "denied by operator")
        if fail_closed:
            return finish(
                EXIT_PERMISSION,
                "consent required; unattended operation fails closed",
            )

        allowlist = sorted(
            {
                AllowRule(
                    resource=description.resource_named(req.resource).id,  # type: ignore[union-attr]
                    offset=req.offset,
                    width=req.width,
                    access=req.access,
                )
                for _, req in requests
            },
            key=lambda rule: (rule.resource, rule.offset, rule.access.value),
        )

        # Execute: open the session with the exact allowlist and session mode,
        # run the bounded operations in order, then drain audit before closing.
        mutating = is_mutating_plan(canonical)
        session_mode = (
            SessionMode.MUTATING if mutating else SessionMode.READ_ONLY
        )
        context = SessionContext(
            run_id=canonical["run_id"],
            case_id=canonical["case_id"],
            plan_digest=digest,
        )
        try:
            session = await self._transport.open_session(
                context,
                description.expectations(),
                allowlist,
                mode=session_mode,
            )
        except OpenSessionRejected as error:
            category = (
                EXIT_STALE
                if error.reason in STALE_REJECTIONS
                else EXIT_ACTIVATION
            )
            return finish(category, f"session rejected: {error.reason.value}")
        except TransportError as error:
            return finish(EXIT_TRANSPORT, f"open_session failed: {error}")

        exit_category = EXIT_SUCCESS
        failure: str | None = None
        operation_rows: list[dict[str, object]] = []
        audit_rows: list[dict[str, object]] = []
        cursor = 0

        async def drain_audit() -> None:
            nonlocal cursor
            while True:
                page = await session.read_audit(cursor, 64)
                if not page.entries:
                    break
                audit_rows.extend(entry.to_json() for entry in page.entries)
                cursor = page.next_cursor

        for operation in operations:
            try:
                if isinstance(operation, _ReadOp):
                    read_outcome = await session.read32(
                        operation.resource.id, operation.offset
                    )
                    operation_rows.append(
                        {
                            "operation": operation.index,
                            "kind": "mmio_read32",
                            "resource": operation.resource.name,
                            "offset": operation.offset,
                            "value": read_outcome.value,
                            "audit_seq": read_outcome.audit_seq,
                            "timestamp_ns": read_outcome.timestamp_ns,
                        }
                    )
                    reads.append(
                        ReadRecord(
                            operation_index=operation.index,
                            resource=operation.resource.name,
                            offset=operation.offset,
                            value=read_outcome.value,
                            audit_seq=read_outcome.audit_seq,
                            timestamp_ns=read_outcome.timestamp_ns,
                        )
                    )
                elif isinstance(operation, _SnapshotOp):
                    items = [
                        SnapshotItem(resource=resource.id, offset=offset)
                        for resource, offset in operation.items
                    ]
                    snapshot = await session.snapshot(items)
                    operation_rows.append(
                        {
                            "operation": operation.index,
                            "kind": "mmio_snapshot32",
                            "complete": snapshot.complete,
                            "results": [
                                {
                                    "resource": result.item.resource,
                                    "offset": result.item.offset,
                                    "ok": result.ok,
                                    "value": (
                                        result.value if result.ok else None
                                    ),
                                    "audit_seq": result.audit_seq,
                                    "timestamp_ns": result.timestamp_ns,
                                }
                                for result in snapshot.results
                            ],
                        }
                    )
                    if not snapshot.complete:
                        exit_category = EXIT_OPERATION
                        failure = "snapshot stopped at a backend failure"
                        break
                elif isinstance(operation, _WriteOp):
                    write_outcome = await session.write32(
                        operation.resource.id,
                        operation.offset,
                        operation.value,
                        write_mask=operation.write_mask,
                        precondition=operation.precondition,
                        readback=operation.readback,
                    )
                    operation_rows.append(
                        {
                            "operation": operation.index,
                            "kind": "mmio_write32",
                            "resource": operation.resource.name,
                            "offset": operation.offset,
                            "value": operation.value,
                            "write_mask": operation.write_mask,
                            "readback_value": write_outcome.readback_value,
                            "audit_seq": write_outcome.audit_seq,
                            "timestamp_ns": write_outcome.timestamp_ns,
                        }
                    )
                    writes.append(
                        WriteRecord(
                            operation_index=operation.index,
                            resource=operation.resource.name,
                            offset=operation.offset,
                            value=operation.value,
                            write_mask=operation.write_mask,
                            readback_value=write_outcome.readback_value,
                            audit_seq=write_outcome.audit_seq,
                            timestamp_ns=write_outcome.timestamp_ns,
                        )
                    )
                    # Spec 14.2 step 6: drain audit immediately after each write.
                    await drain_audit()
                elif isinstance(operation, _PollOp):
                    poll_outcome = await session.poll32(
                        operation.resource.id,
                        operation.offset,
                        operation.expected,
                        mask=operation.mask,
                        interval_ns=operation.interval_ns,
                        timeout_ns=operation.timeout_ns,
                    )
                    operation_rows.append(
                        {
                            "operation": operation.index,
                            "kind": "mmio_poll32",
                            "resource": operation.resource.name,
                            "offset": operation.offset,
                            "expected": operation.expected,
                            "mask": operation.mask,
                            "value": poll_outcome.value,
                            "audit_seq": poll_outcome.audit_seq,
                            "timestamp_ns": poll_outcome.timestamp_ns,
                        }
                    )
                    polls.append(
                        PollRecord(
                            operation_index=operation.index,
                            resource=operation.resource.name,
                            offset=operation.offset,
                            expected=operation.expected,
                            mask=operation.mask,
                            value=poll_outcome.value,
                            audit_seq=poll_outcome.audit_seq,
                            timestamp_ns=poll_outcome.timestamp_ns,
                        )
                    )
                elif isinstance(operation, _SequenceOp):
                    seq_outcome = await session.execute_sequence(
                        operation.items
                    )
                    item_records = [
                        SequenceItemRecord(
                            index=res.index,
                            ok=res.ok,
                            kind=res.kind,
                            value=res.value,
                            readback_value=res.readback_value,
                            audit_seq=res.audit_seq,
                            timestamp_ns=res.timestamp_ns,
                            error=res.error.value
                            if res.error is not None
                            else None,
                        )
                        for res in seq_outcome.results
                    ]
                    seq_record = SequenceRecord(
                        operation_index=operation.index,
                        results=tuple(item_records),
                        complete=seq_outcome.complete,
                    )
                    sequences.append(seq_record)
                    operation_rows.append(
                        {
                            "operation": operation.index,
                            "kind": "sequence",
                            "complete": seq_outcome.complete,
                            "results": [
                                {
                                    "index": r.index,
                                    "ok": r.ok,
                                    "kind": r.kind,
                                    "value": r.value,
                                    "readback_value": r.readback_value,
                                    "audit_seq": r.audit_seq,
                                    "timestamp_ns": r.timestamp_ns,
                                    "error": r.error,
                                }
                                for r in item_records
                            ],
                        }
                    )
                    if operation.has_mutation:
                        # Spec 14.2 step 6: drain audit immediately after mutating sequence.
                        await drain_audit()
                    if not seq_outcome.complete:
                        exit_category = EXIT_OPERATION
                        failure = "sequence stopped at an item failure"
                        break
            except OperationDenied as error:
                kind = "unknown"
                if isinstance(operation, _ReadOp):
                    kind = "mmio_read32"
                elif isinstance(operation, _SnapshotOp):
                    kind = "mmio_snapshot32"
                elif isinstance(operation, _WriteOp):
                    kind = "mmio_write32"
                elif isinstance(operation, _PollOp):
                    kind = "mmio_poll32"
                elif isinstance(operation, _SequenceOp):
                    kind = "sequence"
                operation_rows.append(
                    {
                        "operation": operation.index,
                        "kind": kind,
                        "error": error.denial.value,
                    }
                )
                exit_category = EXIT_OPERATION
                failure = error.denial.value
                break
            except TransportError as error:
                kind = "unknown"
                if isinstance(operation, _ReadOp):
                    kind = "mmio_read32"
                elif isinstance(operation, _SnapshotOp):
                    kind = "mmio_snapshot32"
                elif isinstance(operation, _WriteOp):
                    kind = "mmio_write32"
                elif isinstance(operation, _PollOp):
                    kind = "mmio_poll32"
                elif isinstance(operation, _SequenceOp):
                    kind = "sequence"
                operation_rows.append(
                    {
                        "operation": operation.index,
                        "kind": kind,
                        "error": str(error),
                    }
                )
                exit_category = EXIT_TRANSPORT
                failure = f"operation failed: {error}"
                break
            except (KeyboardInterrupt, asyncio.CancelledError):
                # Operator cancellation stops execution but never skips
                # audit draining, session close, or evidence finalization.
                exit_category = EXIT_OPERATION
                failure = "cancelled by operator"
                break
        recorder.write_jsonl("operations.jsonl", operation_rows)

        # Drain audit and close even after an operation failure or a
        # cancellation so partial runs still produce complete evidence.
        try:
            await drain_audit()
            await session.close()
        except (
            TransportError,
            KeyboardInterrupt,
            asyncio.CancelledError,
        ) as error:
            if failure is None:
                exit_category = EXIT_TRANSPORT
                failure = f"audit drain failed: {error}"
        recorder.write_jsonl("target-audit.jsonl", audit_rows)

        return finish(exit_category, failure)


async def connect(
    target: str = "default",
    *,
    transport: str = "auto",
    timeout_s: float = 10.0,
    **kwargs: Any,
) -> DriverLab:
    """Connects to a target, returning a configured DriverLab instance (Spec Section 9)."""
    return await DriverLab.connect(
        target=target, transport=transport, timeout_s=timeout_s, **kwargs
    )
