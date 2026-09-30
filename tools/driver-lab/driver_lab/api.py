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
from collections.abc import Callable, Mapping
from pathlib import Path
from typing import Any

from driver_lab.consent import (
    READ_WARNING,
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
from driver_lab.plans import plan_digest, validate_plan
from driver_lab.transport import (
    STALE_REJECTIONS,
    AllowRule,
    DirectTransport,
    OpenSessionRejected,
    OperationDenied,
    ProxyDescription,
    ProxyTransport,
    ResourceInfo,
    SessionContext,
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
class RunResult:
    """The outcome of one plan run. Evidence is always finalized."""

    exit_category: int
    evidence_dir: Path
    plan_digest: str
    reads: tuple[ReadRecord, ...] = ()
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

    def _derive(
        self, canonical: Mapping[str, Any], description: ProxyDescription
    ) -> tuple[list[_ReadOp | _SnapshotOp], list[tuple[int, AccessRequest]]]:
        operations: list[_ReadOp | _SnapshotOp] = []
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
            if operation["kind"] == "mmio_read32":
                resource = lookup(operation["resource"])
                offset = operation["offset"]
                operations.append(
                    _ReadOp(index=index, resource=resource, offset=offset)
                )
                requests.append(
                    (index, request(resource, offset, AccessClass.READ_ONCE))
                )
            else:
                items = tuple(
                    (lookup(item["resource"]), item["offset"])
                    for item in operation["items"]
                )
                operations.append(_SnapshotOp(index=index, items=items))
                requests.extend(
                    (index, request(resource, offset, AccessClass.SNAPSHOT))
                    for resource, offset in items
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
                op["kind"] in ("mmio_read32", "mmio_snapshot32")
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
                decision = await self._consent.request_consent(
                    access_request, READ_WARNING
                )
                prompted[access_request.match_key] = decision
                if decision in (
                    ConsentDecision.ALWAYS_ALLOW,
                    ConsentDecision.ALWAYS_DENY,
                ):
                    try:
                        add_grant(
                            self._grants_path,
                            grant_from_decision(access_request, decision),
                        )
                    except GrantStoreError as error:
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

        # Execute: open the session with the exact allowlist, run the
        # bounded operations in order, then drain audit before closing.
        context = SessionContext(
            run_id=canonical["run_id"],
            case_id=canonical["case_id"],
            plan_digest=digest,
        )
        try:
            session = await self._transport.open_session(
                context, description.expectations(), allowlist
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
        for operation in operations:
            try:
                if isinstance(operation, _ReadOp):
                    outcome = await session.read32(
                        operation.resource.id, operation.offset
                    )
                    operation_rows.append(
                        {
                            "operation": operation.index,
                            "kind": "mmio_read32",
                            "resource": operation.resource.name,
                            "offset": operation.offset,
                            "value": outcome.value,
                            "audit_seq": outcome.audit_seq,
                            "timestamp_ns": outcome.timestamp_ns,
                        }
                    )
                    reads.append(
                        ReadRecord(
                            operation_index=operation.index,
                            resource=operation.resource.name,
                            offset=operation.offset,
                            value=outcome.value,
                            audit_seq=outcome.audit_seq,
                            timestamp_ns=outcome.timestamp_ns,
                        )
                    )
                else:
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
                                    "value": result.value
                                    if result.ok
                                    else None,
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
            except OperationDenied as error:
                operation_rows.append(
                    {
                        "operation": operation.index,
                        "kind": (
                            "mmio_read32"
                            if isinstance(operation, _ReadOp)
                            else "mmio_snapshot32"
                        ),
                        "error": error.denial.value,
                    }
                )
                exit_category = EXIT_OPERATION
                failure = error.denial.value
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
        audit_rows: list[dict[str, object]] = []
        cursor = 0
        try:
            while True:
                page = await session.read_audit(cursor, 64)
                if not page.entries:
                    break
                audit_rows.extend(entry.to_json() for entry in page.entries)
                cursor = page.next_cursor
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
