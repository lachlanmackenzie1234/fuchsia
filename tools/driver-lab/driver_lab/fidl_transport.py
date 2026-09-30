# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

"""ProxyTransport implementation over the `fuchsia.driver.lab` FIDL
bindings via fuchsia-controller.

The adapter is connection-agnostic: it takes an already-connected
`Proxy` client and a channel factory, so the same code serves
host-side round-trip tests (in-process channel pairs) and a real target
(channels from a device-connected fuchsia-controller context).
"""

from __future__ import annotations

from collections.abc import Callable, Sequence
from typing import Any

import fidl_fuchsia_driver_lab as fdl
from driver_lab.models import AccessClass
from driver_lab.transport import (
    AllowRule,
    AuditEntry,
    AuditPage,
    Denial,
    Expectations,
    OpenRejection,
    OpenSessionRejected,
    OperationDenied,
    ProxyDescription,
    ReadOutcome,
    ResourceInfo,
    SessionContext,
    SnapshotItem,
    SnapshotItemOutcome,
    SnapshotOutcome,
    TransportError,
)

_ACCESS_TO_FIDL = {
    AccessClass.READ_ONCE: fdl.AccessClass.READ_ONCE,
    AccessClass.SNAPSHOT: fdl.AccessClass.SNAPSHOT,
}


def _unwrap(result: Any) -> Any:
    """Unwraps a flexible-method result wrapper when one is present."""
    unwrap = getattr(result, "unwrap", None)
    if callable(unwrap):
        return unwrap()
    return result


def _enum_lower(value: Any) -> str | None:
    if value is None:
        return None
    name = getattr(value, "name", None)
    if name is None:
        return str(value)
    return str(name).lower()


def _enum_name(enum_type: Any, value: Any) -> str | None:
    """Decoded enum fields may arrive as raw ints; rehydrate before naming."""
    if value is None:
        return None
    if not hasattr(value, "name"):
        value = enum_type(value)
    return _enum_lower(value)


def _to_denial(error: Any) -> Denial:
    if isinstance(error, int):
        error = fdl.OperationError(error)
    name = _enum_lower(error)
    try:
        return Denial(name)
    except ValueError as exc:
        raise TransportError(
            f"unknown operation error from target: {error!r}"
        ) from exc


def _to_rejection(error: Any) -> OpenRejection:
    if isinstance(error, int):
        error = fdl.OpenSessionError(error)
    name = _enum_lower(error)
    try:
        return OpenRejection(name)
    except ValueError as exc:
        raise TransportError(
            f"unknown open-session error from target: {error!r}"
        ) from exc


def _check_denied(result: Any) -> None:
    error = getattr(result, "err", None)
    if error is not None:
        raise OperationDenied(_to_denial(error))


def connect_transport(
    moniker: str,
    capability: str = "fuchsia.driver.lab.Service/default/proxy",
    target: str | None = None,
    config: dict[str, str] | None = None,
) -> "FidlProxyTransport":
    """Connects to the proxy served by a target device.

    Establishes a fuchsia-controller `Context` (the same transport ffx
    uses), connects to `capability` at the driver component `moniker`,
    and returns the adapter. The adapter itself is exercised through
    real FIDL encoding in host tests; this device connection path is
    validated by the emulator conformance suite.
    """
    from fuchsia_controller_py import Context

    context = Context(config=config, target=target)
    channel = context.connect_device_proxy(moniker, capability)
    return FidlProxyTransport(fdl.ProxyClient(channel), context.channel_create)


class FidlProxyTransport:
    """`ProxyTransport` over generated `fuchsia.driver.lab` bindings."""

    def __init__(
        self,
        proxy: Any,
        channel_factory: Callable[[], tuple[Any, Any]],
    ) -> None:
        """`proxy` is a connected `ProxyClient`; `channel_factory`
        returns a (client, server) channel pair in the same handle domain
        as the proxy connection."""
        self._probe = proxy
        self._channel_factory = channel_factory

    async def describe(self) -> ProxyDescription:
        """See `ProxyTransport.describe`."""
        description = _unwrap(await self._probe.describe())
        resources = tuple(
            ResourceInfo(
                id=resource.id_ or 0,
                name=resource.name or "",
                logical_size=resource.logical_size or 0,
                digest=resource.digest or "",
            )
            for resource in (description.resources or [])
        )
        return ProxyDescription(
            protocol_major=description.protocol_major or 0,
            protocol_minor=description.protocol_minor or 0,
            proxy_generation=description.proxy_generation or 0,
            boot_id=description.boot_id or "",
            resource_digest=description.resource_digest or "",
            policy_digest=description.policy_digest or "",
            resources=resources,
            max_snapshot_items=description.max_snapshot_items or 0,
            audit_capacity=description.audit_capacity or 0,
            node_moniker=description.node_moniker,
            topology_generation=description.topology_generation,
            takeover=_enum_name(fdl.TakeoverState, description.takeover),
        )

    async def open_session(
        self,
        context: SessionContext,
        expectations: Expectations,
        allowlist: Sequence[AllowRule],
    ) -> "_FidlProxySession":
        """See `ProxyTransport.open_session`."""
        client_channel, server_channel = self._channel_factory()
        result = await self._probe.open_session(
            context=fdl.RunContext(
                run_id=context.run_id,
                case_id=context.case_id,
                plan_digest=context.plan_digest,
                host_tool_version=context.host_tool_version,
            ),
            mode=fdl.SessionMode.READ_ONLY,
            expectations=fdl.Expectations(
                boot_id=expectations.boot_id,
                proxy_generation=expectations.proxy_generation,
                resource_digest=expectations.resource_digest,
                policy_digest=expectations.policy_digest,
            ),
            allowlist=[
                fdl.AccessRule(
                    resource=rule.resource,
                    offset=rule.offset,
                    width=rule.width,
                    class_=_ACCESS_TO_FIDL[rule.access],
                )
                for rule in allowlist
            ],
            session=server_channel.take(),
        )
        error = getattr(result, "err", None)
        if error is not None:
            raise OpenSessionRejected(_to_rejection(error))
        return _FidlProxySession(fdl.SessionClient(client_channel))


class _FidlProxySession:
    """`ProxySession` over a `Session` client channel."""

    def __init__(self, session: Any) -> None:
        self._session = session

    async def read32(self, resource: int, offset: int) -> ReadOutcome:
        """See `ProxySession.read32`."""
        result = await self._session.read32(resource=resource, offset=offset)
        _check_denied(result)
        response = _unwrap(result)
        return ReadOutcome(
            value=response.value,
            audit_seq=response.audit_seq,
            timestamp_ns=response.timestamp_ns,
        )

    async def snapshot(self, items: Sequence[SnapshotItem]) -> SnapshotOutcome:
        """See `ProxySession.snapshot`."""
        result = await self._session.snapshot(
            items=[
                fdl.SnapshotItem(resource=item.resource, offset=item.offset)
                for item in items
            ]
        )
        _check_denied(result)
        response = _unwrap(result)
        return SnapshotOutcome(
            results=tuple(
                SnapshotItemOutcome(
                    item=SnapshotItem(
                        resource=entry.item.resource, offset=entry.item.offset
                    ),
                    ok=entry.ok,
                    value=entry.value,
                    audit_seq=entry.audit_seq,
                    timestamp_ns=entry.timestamp_ns,
                )
                for entry in response.results
            ),
            complete=response.complete,
        )

    async def read_audit(self, cursor: int, limit: int) -> AuditPage:
        """See `ProxySession.read_audit`."""
        response = _unwrap(
            await self._session.read_audit(cursor=cursor, limit=limit)
        )
        entries = tuple(
            AuditEntry(
                seq=entry.seq or 0,
                operation=entry.operation or "",
                session=entry.session,
                resource=entry.resource,
                offset=entry.offset,
                decision=_enum_name(fdl.AuditDecision, entry.decision)
                or "allowed",
                denial=_enum_name(fdl.OperationError, entry.denial),
                status=_enum_name(fdl.AuditStatus, entry.status) or "ok",
                value=entry.value,
                timestamp_ns=entry.timestamp_ns or 0,
                run_id=entry.run_id,
                item_index=entry.item_index,
                proxy_generation=entry.proxy_generation,
                boot_id=entry.boot_id,
            )
            for entry in response.entries
        )
        oldest = response.oldest_retained if response.has_oldest else None
        return AuditPage(
            entries=entries,
            oldest_retained=oldest,
            next_cursor=response.next_cursor,
        )

    async def close(self) -> None:
        """See `ProxySession.close`. Dropping the client closes the channel."""
        self._session = None
