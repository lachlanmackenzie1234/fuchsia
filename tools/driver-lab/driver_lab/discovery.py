# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

"""Node discovery via fuchsia.driver.development.Manager.

Provides stable node enumeration, bound driver inspection, and unclaimed-node
detection over FIDL, fulfilling Phase 1 Milestone H0/H1.
"""

from __future__ import annotations

import asyncio
import dataclasses
from collections.abc import Callable, Mapping
from typing import Any, Protocol

import fidl_fuchsia_driver_development as fdd


class DiscoveryError(Exception):
    """Base exception for driver node discovery failures."""


class NodeNotFoundError(DiscoveryError):
    """The requested node moniker was not found."""


class DiscoveryTransportError(DiscoveryError):
    """Underlying FIDL or transport failure during node discovery."""


@dataclasses.dataclass(frozen=True)
class NodeSummary:
    """Stable summary of a discovered driver-framework node."""

    moniker: str
    bound_driver_url: str | None = None
    driver_host_koid: int | None = None
    properties: Mapping[str, Any] = dataclasses.field(default_factory=dict)
    offers: tuple[str, ...] = ()
    quarantined: bool = False
    topological_path: str | None = None

    @property
    def is_unclaimed(self) -> bool:
        """True if the node has no bound driver."""
        return self.bound_driver_url is None or self.bound_driver_url == ""


@dataclasses.dataclass(frozen=True)
class NodeDescription:
    """Detailed description of a discovered driver-framework node."""

    moniker: str
    bound_driver_url: str | None = None
    driver_host_koid: int | None = None
    properties: Mapping[str, Any] = dataclasses.field(default_factory=dict)
    offers: tuple[str, ...] = ()
    quarantined: bool = False
    topological_path: str | None = None

    @property
    def is_unclaimed(self) -> bool:
        """True if the node has no bound driver."""
        return self.bound_driver_url is None or self.bound_driver_url == ""

    def has_protocol(self, protocol: str) -> bool:
        """True if the node offers the specified protocol."""
        return protocol in self.offers


def _extract_property_value(val: Any) -> Any:
    """Extracts a primitive value from a NodePropertyValue union."""
    if val is None:
        return None
    for attr in ("string_value", "int_value", "bool_value", "enum_value"):
        if hasattr(val, attr):
            v = getattr(val, attr)
            if v is not None:
                return v
    return str(val)


def _extract_properties(node: Any) -> dict[str, Any]:
    """Extracts node properties from NodeInfo into a JSON-friendly dict."""
    props: dict[str, Any] = {}
    prop_list = getattr(node, "node_property_list", None) or []
    for prop in prop_list:
        key = getattr(prop, "key", None)
        key_str = (
            getattr(key, "string_value", None)
            or getattr(key, "int_value", None)
            or str(key)
        )
        val = getattr(prop, "value", None)
        props[str(key_str)] = _extract_property_value(val)
    return props


def _extract_offers(node: Any) -> tuple[str, ...]:
    """Extracts offer service/protocol names from NodeInfo."""
    offer_list = getattr(node, "offer_list", None) or []
    offers = []
    for offer in offer_list:
        target = (
            getattr(offer, "service", None)
            or getattr(offer, "protocol", None)
            or str(offer)
        )
        target_name = getattr(target, "target_name", None) or str(target)
        offers.append(target_name)
    return tuple(offers)


class NodeDiscovery(Protocol):
    """Abstract interface for discovering driver framework nodes."""

    async def list_nodes(
        self,
        node_filter: list[str] | None = None,
        exact_match: bool = False,
    ) -> list[NodeSummary]:
        """Lists nodes matching `node_filter`."""
        ...

    async def describe_node(self, moniker: str) -> NodeDescription | None:
        """Describes a single node by moniker."""
        ...

    async def find_unclaimed(self) -> list[NodeSummary]:
        """Returns all currently unclaimed nodes."""
        ...

    async def find_nodes_offering(self, protocol: str) -> list[NodeSummary]:
        """Returns all nodes offering the specified protocol or service."""
        ...


class FakeNodeDiscovery:
    """In-memory node discovery provider for unit tests."""

    def __init__(self, nodes: list[NodeDescription] | None = None) -> None:
        self._nodes: dict[str, NodeDescription] = {
            n.moniker: n for n in (nodes or [])
        }

    def add_node(self, node: NodeDescription) -> None:
        self._nodes[node.moniker] = node

    def remove_node(self, moniker: str) -> None:
        self._nodes.pop(moniker, None)

    async def list_nodes(
        self,
        node_filter: list[str] | None = None,
        exact_match: bool = False,
    ) -> list[NodeSummary]:
        results: list[NodeSummary] = []
        for moniker, desc in self._nodes.items():
            matched = False
            if not node_filter:
                matched = True
            elif exact_match:
                matched = any(moniker == f for f in node_filter)
            else:
                matched = any(f in moniker for f in node_filter)

            if matched:
                results.append(
                    NodeSummary(
                        moniker=desc.moniker,
                        bound_driver_url=desc.bound_driver_url,
                        driver_host_koid=desc.driver_host_koid,
                        properties=desc.properties,
                        offers=desc.offers,
                        quarantined=desc.quarantined,
                        topological_path=desc.topological_path,
                    )
                )
        return results

    async def describe_node(self, moniker: str) -> NodeDescription | None:
        return self._nodes.get(moniker)

    async def find_unclaimed(self) -> list[NodeSummary]:
        nodes = await self.list_nodes()
        return [n for n in nodes if n.is_unclaimed]

    async def find_nodes_offering(self, protocol: str) -> list[NodeSummary]:
        nodes = await self.list_nodes()
        return [n for n in nodes if protocol in n.offers]


class FidlNodeDiscovery:
    """`NodeDiscovery` implementation over fuchsia.driver.development.Manager."""

    def __init__(
        self,
        manager: Any,
        channel_factory: Callable[[], tuple[Any, Any]],
    ) -> None:
        self._manager = manager
        self._channel_factory = channel_factory

    async def list_nodes(
        self,
        node_filter: list[str] | None = None,
        exact_match: bool = False,
    ) -> list[NodeSummary]:
        client_chan, server_chan = self._channel_factory()
        server_handle = (
            server_chan.take() if hasattr(server_chan, "take") else server_chan
        )
        try:
            res = self._manager.get_node_info(
                node_filter=node_filter or [],
                iterator=server_handle,
                exact_match=exact_match,
            )
            if hasattr(res, "__await__"):
                await res
        except Exception as exc:
            raise DiscoveryTransportError(
                f"get_node_info failed: {exc}"
            ) from exc

        iterator = fdd.NodeInfoIteratorClient(client_chan)
        summaries: list[NodeSummary] = []
        try:
            while True:
                response = await iterator.get_next()
                batch: list[Any] = (
                    getattr(response, "nodes", None)
                    or getattr(response, "response", None)
                    or []
                )
                if not batch:
                    break
                for item in batch:
                    url = (
                        item.bound_driver_url
                        if hasattr(item, "bound_driver_url")
                        else None
                    )
                    if url == "":
                        url = None
                    summaries.append(
                        NodeSummary(
                            moniker=item.moniker or "",
                            bound_driver_url=url,
                            driver_host_koid=getattr(
                                item, "driver_host_koid", None
                            ),
                            properties=_extract_properties(item),
                            offers=_extract_offers(item),
                            quarantined=getattr(item, "quarantined", False)
                            or False,
                            topological_path=getattr(
                                item, "topological_path", None
                            ),
                        )
                    )
        except Exception as exc:
            raise DiscoveryTransportError(
                f"reading NodeInfoIterator failed: {exc}"
            ) from exc

        return summaries

    async def describe_node(self, moniker: str) -> NodeDescription | None:
        client_chan, server_chan = self._channel_factory()
        server_handle = (
            server_chan.take() if hasattr(server_chan, "take") else server_chan
        )
        try:
            res = self._manager.get_node_info(
                node_filter=[moniker],
                iterator=server_handle,
                exact_match=True,
            )
            if hasattr(res, "__await__"):
                await res
        except Exception as exc:
            raise DiscoveryTransportError(
                f"get_node_info failed: {exc}"
            ) from exc

        iterator = fdd.NodeInfoIteratorClient(client_chan)
        found_item = None
        try:
            while True:
                response = await iterator.get_next()
                batch: list[Any] = (
                    getattr(response, "nodes", None)
                    or getattr(response, "response", None)
                    or []
                )
                if not batch:
                    break
                for item in batch:
                    if (item.moniker or "") == moniker:
                        found_item = item
                        break
                if found_item is not None:
                    break
        except Exception as exc:
            raise DiscoveryTransportError(
                f"reading NodeInfoIterator failed: {exc}"
            ) from exc

        if found_item is None:
            return None

        url = (
            found_item.bound_driver_url
            if hasattr(found_item, "bound_driver_url")
            else None
        )
        if url == "":
            url = None

        return NodeDescription(
            moniker=found_item.moniker or "",
            bound_driver_url=url,
            driver_host_koid=getattr(found_item, "driver_host_koid", None),
            properties=_extract_properties(found_item),
            offers=_extract_offers(found_item),
            quarantined=getattr(found_item, "quarantined", False) or False,
            topological_path=getattr(found_item, "topological_path", None),
        )

    async def find_unclaimed(self) -> list[NodeSummary]:
        nodes = await self.list_nodes()
        return [n for n in nodes if n.is_unclaimed]

    async def find_nodes_offering(self, protocol: str) -> list[NodeSummary]:
        nodes = await self.list_nodes()
        return [n for n in nodes if protocol in n.offers]


def connect_discovery(
    target: str | None = None,
    moniker: str = "bootstrap/driver_manager",
    capability: str = "fuchsia.driver.development.Manager",
    config: dict[str, str] | None = None,
) -> FidlNodeDiscovery:
    """Connects to the driver development manager on a Fuchsia target."""
    from fuchsia_controller_py import Context

    context = Context(config=config, target=target)
    channel = context.connect_device_proxy(moniker, capability)
    return FidlNodeDiscovery(fdd.ManagerClient(channel), context.channel_create)


DEFAULT_PROXY_DRIVER_URL = (
    "fuchsia-pkg://fuchsia.com/lab_proxy#meta/lab_proxy.cm"
)


class ProxyActivator(Protocol):
    """Interface for activating and deactivating the proxy driver on unclaimed nodes."""

    async def bind_proxy(
        self,
        node_id: str,
        driver_url: str = DEFAULT_PROXY_DRIVER_URL,
    ) -> None:
        """Binds the proxy driver to the specified unclaimed node."""
        ...

    async def end_proxy(
        self,
        node_id: str,
        driver_url: str = DEFAULT_PROXY_DRIVER_URL,
    ) -> None:
        """Ends proxy access on the specified node."""
        ...


class FakeProxyActivator:
    """In-memory activator for unit tests that mutates FakeNodeDiscovery state."""

    def __init__(self, discovery: FakeNodeDiscovery) -> None:
        self._discovery = discovery
        self.bind_calls: list[tuple[str, str]] = []
        self.end_calls: list[tuple[str, str]] = []

    async def bind_proxy(
        self,
        node_id: str,
        driver_url: str = DEFAULT_PROXY_DRIVER_URL,
    ) -> None:
        self.bind_calls.append((node_id, driver_url))
        node = await self._discovery.describe_node(node_id)
        if node is None:
            raise NodeNotFoundError(f"node {node_id!r} not found")
        self._discovery.add_node(
            NodeDescription(
                moniker=node.moniker,
                bound_driver_url=driver_url,
                driver_host_koid=node.driver_host_koid,
                properties=node.properties,
                offers=node.offers,
                quarantined=node.quarantined,
                topological_path=node.topological_path,
            )
        )

    async def end_proxy(
        self,
        node_id: str,
        driver_url: str = DEFAULT_PROXY_DRIVER_URL,
    ) -> None:
        self.end_calls.append((node_id, driver_url))
        node = await self._discovery.describe_node(node_id)
        if node is None:
            raise NodeNotFoundError(f"node {node_id!r} not found")
        self._discovery.add_node(
            NodeDescription(
                moniker=node.moniker,
                bound_driver_url=None,
                driver_host_koid=node.driver_host_koid,
                properties=node.properties,
                offers=node.offers,
                quarantined=node.quarantined,
                topological_path=node.topological_path,
            )
        )


class FfxProxyActivator:
    """Proxy activator communicating via ffx driver subcommands."""

    def __init__(self, target: str | None = None) -> None:
        self._target = target

    async def bind_proxy(
        self,
        node_id: str,
        driver_url: str = DEFAULT_PROXY_DRIVER_URL,
    ) -> None:
        cmd = ["ffx"]
        if self._target:
            cmd.extend(["--target", self._target])
        cmd.extend(["driver", "register", driver_url])
        proc = await asyncio.create_subprocess_exec(
            *cmd, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE
        )
        _, stderr = await proc.communicate()
        if proc.returncode != 0:
            raise DiscoveryTransportError(
                f"ffx driver register failed ({proc.returncode}): {stderr.decode().strip()}"
            )

    async def end_proxy(
        self,
        node_id: str,
        driver_url: str = DEFAULT_PROXY_DRIVER_URL,
    ) -> None:
        cmd = ["ffx"]
        if self._target:
            cmd.extend(["--target", self._target])
        cmd.extend(["driver", "disable", driver_url])
        proc = await asyncio.create_subprocess_exec(
            *cmd, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE
        )
        _, stderr = await proc.communicate()
        if proc.returncode != 0:
            raise DiscoveryTransportError(
                f"ffx driver disable failed ({proc.returncode}): {stderr.decode().strip()}"
            )


def connect_activator(
    target: str | None = None,
) -> ProxyActivator:
    """Returns a proxy activator for the target."""
    return FfxProxyActivator(target=target)
