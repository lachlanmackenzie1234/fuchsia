# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

"""Round-trip tests for the FIDL transport adapter.

A Python-served `fuchsia.driver.lab` implementation bridges to
`FakeProxyTarget`, so the adapter is exercised through real FIDL
encoding over in-process channel pairs -- no emulator required. This is
also the seed of a conformance suite: the same fake semantics the API
tests rely on are here proven reachable through the wire contract.
"""

import asyncio
import json
import os
import tempfile
import unittest
from pathlib import Path
from typing import Any

import fidl_fuchsia_driver_lab as fdl
from driver_lab.api import EXIT_OPERATION, EXIT_STALE, DriverLab
from driver_lab.fidl_transport import FidlProxyTransport
from driver_lab.models import AccessClass, Decision, ReadGrant
from driver_lab.permissions import save_grants
from driver_lab.transport import (
    AllowRule,
    Expectations,
    FakeProxyTarget,
    OpenRejection,
    OpenSessionRejected,
    ProxyDescription,
    ProxySession,
    ResourceInfo,
    SessionContext,
    SnapshotItem,
)
from fidl import DomainError
from fidl._ipc import GlobalHandleWaker
from fuchsia_controller_py import Channel, Context

CTRL_DIGEST = "sha256:" + "ab" * 32
DESCRIPTION_DIGEST = "sha256:" + "ee" * 32
POLICY_DIGEST = "sha256:" + "ff" * 32
TARGET_SCOPE = "example-engineering-target"
NODE_ID = "example-device"


def make_description() -> ProxyDescription:
    return ProxyDescription(
        protocol_major=1,
        protocol_minor=0,
        proxy_generation=7,
        boot_id="boot-1",
        resource_digest=DESCRIPTION_DIGEST,
        policy_digest=POLICY_DIGEST,
        resources=(
            ResourceInfo(
                id=1, name="control", logical_size=0x100, digest=CTRL_DIGEST
            ),
        ),
        max_snapshot_items=64,
        audit_capacity=1024,
    )


class _BridgeSessionServer(fdl.SessionServer):
    """Serves one `Session` channel by delegating to a fake session."""

    def __init__(self, channel: Channel, session: ProxySession) -> None:
        super().__init__(channel)
        self._session = session

    async def read32(self, request: Any) -> Any:
        try:
            outcome = await self._session.read32(
                request.resource, request.offset
            )
        except Exception as error:  # OperationDenied
            denial = getattr(error, "denial", None)
            if denial is None:
                raise
            return DomainError(error=getattr(fdl.OperationError, denial.name))
        return fdl.ReadResult(
            value=outcome.value,
            audit_seq=outcome.audit_seq,
            timestamp_ns=outcome.timestamp_ns,
        )

    async def snapshot(self, request: Any) -> Any:
        items = [
            SnapshotItem(resource=item.resource, offset=item.offset)
            for item in request.items
        ]
        try:
            outcome = await self._session.snapshot(items)
        except Exception as error:  # OperationDenied
            denial = getattr(error, "denial", None)
            if denial is None:
                raise
            return DomainError(error=getattr(fdl.OperationError, denial.name))
        return fdl.SnapshotResult(
            results=[
                fdl.SnapshotItemResult(
                    item=fdl.SnapshotItem(
                        resource=result.item.resource, offset=result.item.offset
                    ),
                    ok=result.ok,
                    value=result.value,
                    audit_seq=result.audit_seq,
                    timestamp_ns=result.timestamp_ns,
                )
                for result in outcome.results
            ],
            complete=outcome.complete,
        )

    async def read_audit(self, request: Any) -> Any:
        page = await self._session.read_audit(request.cursor, request.limit)
        return fdl.AuditPage(
            entries=[
                fdl.AuditEntry(
                    seq=entry.seq,
                    session=entry.session,
                    resource=entry.resource,
                    operation=entry.operation,
                    offset=entry.offset,
                    decision=getattr(fdl.AuditDecision, entry.decision.upper()),
                    denial=(
                        getattr(fdl.OperationError, entry.denial.upper())
                        if entry.denial
                        else None
                    ),
                    status=getattr(fdl.AuditStatus, entry.status.upper()),
                    value=entry.value,
                    timestamp_ns=entry.timestamp_ns,
                    run_id=entry.run_id,
                    item_index=entry.item_index,
                    proxy_generation=entry.proxy_generation,
                    boot_id=entry.boot_id,
                )
                for entry in page.entries
            ],
            has_oldest=page.oldest_retained is not None,
            oldest_retained=page.oldest_retained or 0,
            next_cursor=page.next_cursor,
        )

    async def write32(self, request: Any) -> Any:
        if hasattr(self._session, "write32"):
            return await self._session.write32(request)
        return DomainError(error=fdl.OperationError.NOT_PERMITTED_BY_CEILING)

    async def poll32(self, request: Any) -> Any:
        if hasattr(self._session, "poll32"):
            return await self._session.poll32(request)
        return DomainError(error=fdl.OperationError.NOT_PERMITTED_BY_CEILING)

    async def execute_sequence(self, request: Any) -> Any:
        if hasattr(self._session, "execute_sequence"):
            return await self._session.execute_sequence(request)
        return DomainError(error=fdl.OperationError.NOT_PERMITTED_BY_CEILING)


class _BridgeProxyServer(fdl.ProxyServer):
    """Serves `Proxy` by delegating to a `FakeProxyTarget`."""

    def __init__(
        self,
        channel: Channel,
        fake: FakeProxyTarget,
        tasks: list[asyncio.Task[None]],
    ) -> None:
        super().__init__(channel)
        self._fake = fake
        self._tasks = tasks

    async def describe(self) -> Any:
        description = await self._fake.describe()
        return fdl.ProxyDescription(
            protocol_major=description.protocol_major,
            protocol_minor=description.protocol_minor,
            proxy_generation=description.proxy_generation,
            boot_id=description.boot_id,
            resource_digest=description.resource_digest,
            policy_digest=description.policy_digest,
            resources=[
                fdl.ResourceDescription(
                    id_=resource.id,
                    name=resource.name,
                    kind=fdl.ResourceKind.MMIO,
                    logical_size=resource.logical_size,
                    digest=resource.digest,
                )
                for resource in description.resources
            ],
            max_snapshot_items=description.max_snapshot_items,
            audit_capacity=description.audit_capacity,
        )

    async def open_session(self, request: Any) -> Any:
        # Mirror the real driver: reserved phase 2 expectations fail
        # closed, and an absent required field falls through to the
        # per-field stale check.
        if (
            request.expectations.topology_generation is not None
            or request.expectations.bound_driver_url is not None
        ):
            return DomainError(
                error=fdl.OpenSessionError.UNSUPPORTED_EXPECTATION
            )
        expectations = Expectations(
            boot_id=request.expectations.boot_id or "",
            proxy_generation=request.expectations.proxy_generation or 0,
            resource_digest=request.expectations.resource_digest or "",
            policy_digest=request.expectations.policy_digest or "",
        )
        allowlist = [
            AllowRule(
                resource=rule.resource,
                offset=rule.offset,
                width=rule.width,
                # Decoded enum fields may arrive as raw ints.
                access=AccessClass(fdl.AccessClass(rule.class_).name.lower()),
            )
            for rule in request.allowlist
        ]
        context = SessionContext(
            run_id=request.context.run_id or "",
            case_id=request.context.case_id or "",
            plan_digest=request.context.plan_digest or "",
        )
        try:
            session = await self._fake.open_session(
                context, expectations, allowlist
            )
        except OpenSessionRejected as error:
            return DomainError(
                error=getattr(fdl.OpenSessionError, error.reason.name)
            )
        channel = request.session
        if not isinstance(channel, Channel):
            channel = Channel(channel)
        server = _BridgeSessionServer(channel, session)
        self._tasks.append(
            asyncio.get_running_loop().create_task(server.serve())
        )
        return fdl.ProxyOpenSessionResponse(session_id=1)


def make_grant(**overrides: object) -> ReadGrant:
    fields: dict[str, Any] = dict(
        schema_version=1,
        target_scope=TARGET_SCOPE,
        node_id=NODE_ID,
        resource_digest=CTRL_DIGEST,
        resource="control",
        offset=0x3C,
        width=4,
        access=AccessClass.READ_ONCE,
        decision=Decision.ALLOW,
        approved_at="2026-07-30T00:00:00Z",
    )
    fields.update(overrides)
    return ReadGrant(**fields)


def make_plan(**overrides: object) -> dict[str, Any]:
    plan: dict[str, Any] = {
        "schema_version": 1,
        "run_id": "run-1",
        "case_id": "case-1",
        "target": {"selector": "lab-target"},
        "node": {
            "id": NODE_ID,
            "expected_resource_digest": DESCRIPTION_DIGEST,
        },
        "access": {"mode": "proxy", "activation": "bind-unclaimed"},
        "operations": [
            {"kind": "mmio_read32", "resource": "control", "offset": "0x3c"},
        ],
    }
    plan.update(overrides)
    return plan


class FidlRoundTripTest(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        GlobalHandleWaker()._reset_for_testing()
        self._dir = tempfile.TemporaryDirectory()
        self.addCleanup(self._dir.cleanup)
        base = Path(self._dir.name)
        self.grants_path = base / "grants.toml"
        self.evidence_root = base / "evidence"
        self.fake = FakeProxyTarget(make_description())
        self.fake.set_value(1, 0x3C, 0xDEAD_BEEF)
        self.tasks: list[asyncio.Task[None]] = []
        self._orig_nodename = os.environ.pop("FUCHSIA_NODENAME", None)
        self._orig_device_addr = os.environ.pop("FUCHSIA_DEVICE_ADDR", None)

    async def asyncTearDown(self) -> None:
        for task in self.tasks:
            if task.done() and not task.cancelled():
                exception = task.exception()
                if exception is not None:
                    import traceback

                    traceback.print_exception(exception)
            task.cancel()
        if self._orig_nodename is not None:
            os.environ["FUCHSIA_NODENAME"] = self._orig_nodename
        if self._orig_device_addr is not None:
            os.environ["FUCHSIA_DEVICE_ADDR"] = self._orig_device_addr

    def make_lab(self) -> DriverLab:
        context = Context(target="")
        (client_channel, server_channel) = context.channel_create()
        self._context = context
        proxy_server = _BridgeProxyServer(server_channel, self.fake, self.tasks)
        self.tasks.append(
            asyncio.get_running_loop().create_task(proxy_server.serve())
        )
        transport = FidlProxyTransport(
            fdl.ProxyClient(client_channel), context.channel_create
        )
        return DriverLab(
            transport,
            grants_path=self.grants_path,
            evidence_root=self.evidence_root,
            target_scope=TARGET_SCOPE,
            node_id=NODE_ID,
        )

    async def test_describe_round_trips(self) -> None:
        context = Context(target="")
        (client_channel, server_channel) = context.channel_create()
        proxy_server = _BridgeProxyServer(server_channel, self.fake, self.tasks)
        self.tasks.append(
            asyncio.get_running_loop().create_task(proxy_server.serve())
        )
        adapter = FidlProxyTransport(
            fdl.ProxyClient(client_channel), context.channel_create
        )
        description = await adapter.describe()
        self.assertEqual(description, make_description())

    async def test_run_plan_over_real_fidl(self) -> None:
        save_grants(self.grants_path, [make_grant()])
        lab = self.make_lab()
        result = await lab.run_plan(make_plan())
        self.assertTrue(result.ok, result.failure)
        self.assertEqual(result.reads[0].value, 0xDEAD_BEEF)
        audit_ops = [
            json.loads(line)["operation"]
            for line in (result.evidence_dir / "target-audit.jsonl")
            .read_text()
            .splitlines()
        ]
        self.assertIn("open_session", audit_ops)
        self.assertIn("read32", audit_ops)

    async def test_backend_fault_over_fidl(self) -> None:
        save_grants(self.grants_path, [make_grant()])
        self.fake.fail_at(1, 0x3C)
        lab = self.make_lab()
        result = await lab.run_plan(make_plan())
        self.assertEqual(result.exit_category, EXIT_OPERATION)
        self.assertEqual(result.failure, "backend_fault")

    async def test_session_rejection_over_fidl(self) -> None:
        save_grants(self.grants_path, [make_grant()])
        self.fake.reject_open = OpenRejection.STALE_PROXY_GENERATION
        lab = self.make_lab()
        result = await lab.run_plan(make_plan())
        self.assertEqual(result.exit_category, EXIT_STALE)
        self.assertEqual(
            result.failure, "session rejected: stale_proxy_generation"
        )


if __name__ == "__main__":
    unittest.main()
