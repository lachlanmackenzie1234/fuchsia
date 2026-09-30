# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

"""Plan validation, canonicalization, and digests.

Plans are data, not code: no expressions, no arbitrary driver URLs,
component monikers, or protocol paths. Validation rejects unknown keys
everywhere so nothing can be smuggled past review. Canonicalization
normalizes numeric representations and key order -- operation order is
semantic and preserved -- so any change to an executable field changes the
digest and invalidates approval.

The schema carries the phase 2 (managed takeover) keys as reserved:
setting one fails with a distinct reserved-key error, so phase 2 can
adopt them without a schema-shape change.
"""

from __future__ import annotations

import hashlib
import json
from collections.abc import Mapping
from typing import Any

SCHEMA_VERSION = 1

_TOP_KEYS = {
    "schema_version",
    "run_id",
    "case_id",
    "target",
    "node",
    "access",
    "operations",
}
_TARGET_KEYS = {"selector", "expected_boot_id"}
_NODE_KEYS = {"id", "expected_unclaimed", "expected_resource_digest"} | {
    "expected_bound_driver_url",
    "expected_topology_generation",
}
_ACCESS_KEYS = {
    "mode",
    "activation",
    "requires_target_policy",
    "requires_target_audit",
    "requires_target_local_timing",
    "restoration",
}
# Keys the schema carries for phase 2 (managed takeover) so their later
# arrival changes no schema shape. A phase 1 plan that sets one fails
# with a reserved-key error, never an unknown-key error.
_RESERVED_PHASE2_KEYS = {
    "node.expected_bound_driver_url",
    "node.expected_topology_generation",
    "access.restoration",
}
_MAX_STRING = 128


class PlanError(Exception):
    """The plan is structurally invalid or requests something unsupported."""


def _require_str(value: object, what: str) -> str:
    if not isinstance(value, str) or not value or len(value) > _MAX_STRING:
        raise PlanError(
            f"{what} must be a non-empty string of at most {_MAX_STRING} characters"
        )
    return value


def _require_bool(value: object, what: str) -> bool:
    if not isinstance(value, bool):
        raise PlanError(f"{what} must be a boolean")
    return value


def _require_keys(
    mapping: Mapping[str, object], allowed: set[str], what: str
) -> None:
    if not isinstance(mapping, Mapping):
        raise PlanError(f"{what} must be an object")
    unknown = set(mapping) - allowed
    if unknown:
        raise PlanError(f"{what} has unknown keys: {sorted(unknown)}")
    reserved = {
        key for key in mapping if f"{what}.{key}" in _RESERVED_PHASE2_KEYS
    }
    if reserved:
        raise PlanError(
            f"{what} keys {sorted(reserved)} are reserved until phase 2 (managed takeover)"
        )


def _parse_offset(value: object, what: str) -> int:
    if isinstance(value, bool):
        raise PlanError(f"{what} must be an integer or hex string")
    if isinstance(value, int):
        offset = value
    elif isinstance(value, str):
        text = value.strip().lower()
        try:
            offset = int(text, 16) if text.startswith("0x") else int(text, 10)
        except ValueError as error:
            raise PlanError(
                f"{what} is not a valid offset: {value!r}"
            ) from error
    else:
        raise PlanError(f"{what} must be an integer or hex string")
    if offset < 0:
        raise PlanError(f"{what} must be non-negative")
    return offset


def _validate_operation(operation: object, index: int) -> dict[str, Any]:
    what = f"operations[{index}]"
    if not isinstance(operation, Mapping):
        raise PlanError(f"{what} must be an object")
    kind = operation.get("kind")
    if kind == "mmio_read32":
        _require_keys(operation, {"kind", "resource", "offset"}, what)
        return {
            "kind": "mmio_read32",
            "resource": _require_str(
                operation.get("resource"), f"{what}.resource"
            ),
            "offset": _parse_offset(operation.get("offset"), f"{what}.offset"),
        }
    if kind == "mmio_snapshot32":
        _require_keys(operation, {"kind", "items"}, what)
        items = operation.get("items")
        if not isinstance(items, list) or not items:
            raise PlanError(f"{what}.items must be a non-empty list")
        canonical_items = []
        for item_index, item in enumerate(items):
            item_what = f"{what}.items[{item_index}]"
            if not isinstance(item, Mapping):
                raise PlanError(f"{item_what} must be an object")
            _require_keys(item, {"resource", "offset"}, item_what)
            canonical_items.append(
                {
                    "resource": _require_str(
                        item.get("resource"), f"{item_what}.resource"
                    ),
                    "offset": _parse_offset(
                        item.get("offset"), f"{item_what}.offset"
                    ),
                }
            )
        return {"kind": "mmio_snapshot32", "items": canonical_items}
    if kind == "fidl_call":
        _require_keys(operation, {"kind", "method", "args"}, what)
        method = _require_str(operation.get("method"), f"{what}.method")
        args = operation.get("args")
        if args is not None and not isinstance(args, Mapping):
            raise PlanError(f"{what}.args must be an object")
        canonical_args: dict[str, Any] = {}
        if isinstance(args, Mapping):
            for k, v in args.items():
                if not isinstance(k, str):
                    raise PlanError(f"{what}.args keys must be strings")
                canonical_args[k] = v
        return {
            "kind": "fidl_call",
            "method": method,
            "args": canonical_args,
        }
    raise PlanError(f"{what}.kind is unsupported: {kind!r}")


def validate_plan(plan: Mapping[str, object]) -> dict[str, Any]:
    """Validates `plan` and returns its canonical structure.

    Rejects unknown keys at every level, unsupported access modes and
    activations (managed takeover is phase 2), and malformed operations.
    """
    _require_keys(plan, _TOP_KEYS, "plan")
    if plan.get("schema_version") != SCHEMA_VERSION:
        raise PlanError("schema_version must be 1")

    run_id = _require_str(plan.get("run_id"), "run_id")
    case_id = _require_str(plan.get("case_id"), "case_id")

    target_in = plan.get("target")
    _require_keys(target_in, _TARGET_KEYS, "target")  # type: ignore[arg-type]
    assert isinstance(target_in, Mapping)
    target: dict[str, Any] = {
        "selector": _require_str(target_in.get("selector"), "target.selector")
    }
    if "expected_boot_id" in target_in:
        target["expected_boot_id"] = _require_str(
            target_in.get("expected_boot_id"), "target.expected_boot_id"
        )

    node_in = plan.get("node")
    _require_keys(node_in, _NODE_KEYS, "node")  # type: ignore[arg-type]
    assert isinstance(node_in, Mapping)
    node: dict[str, Any] = {"id": _require_str(node_in.get("id"), "node.id")}
    if "expected_unclaimed" in node_in:
        node["expected_unclaimed"] = _require_bool(
            node_in.get("expected_unclaimed"), "node.expected_unclaimed"
        )
    if "expected_resource_digest" in node_in:
        node["expected_resource_digest"] = _require_str(
            node_in.get("expected_resource_digest"),
            "node.expected_resource_digest",
        )

    access_in = plan.get("access")
    _require_keys(access_in, _ACCESS_KEYS, "access")  # type: ignore[arg-type]
    assert isinstance(access_in, Mapping)
    mode = access_in.get("mode")
    if mode not in ("direct", "proxy"):
        raise PlanError(f"access.mode is unsupported: {mode!r}")
    access: dict[str, Any] = {"mode": mode}
    activation = access_in.get("activation")
    if mode == "proxy":
        if activation == "takeover":
            raise PlanError(
                "access.activation 'takeover' is not supported until phase 2"
            )
        if activation not in (None, "bind-unclaimed"):
            raise PlanError(f"access.activation is unsupported: {activation!r}")
        access["activation"] = "bind-unclaimed"
    elif activation is not None:
        raise PlanError("access.activation is only valid in proxy mode")
    for flag in (
        "requires_target_policy",
        "requires_target_audit",
        "requires_target_local_timing",
    ):
        value = access_in.get(flag, False)
        access[flag] = _require_bool(value, f"access.{flag}")

    operations_in = plan.get("operations")
    if not isinstance(operations_in, list) or not operations_in:
        raise PlanError("operations must be a non-empty list")
    operations = [
        _validate_operation(operation, index)
        for index, operation in enumerate(operations_in)
    ]

    return {
        "schema_version": SCHEMA_VERSION,
        "run_id": run_id,
        "case_id": case_id,
        "target": target,
        "node": node,
        "access": access,
        "operations": operations,
    }


def canonical_json(plan: Mapping[str, object]) -> bytes:
    """The canonical byte form of `plan`: validated, keys sorted, numbers
    normalized, operation order preserved."""
    canonical = validate_plan(plan)
    return json.dumps(
        canonical, sort_keys=True, separators=(",", ":"), ensure_ascii=True
    ).encode()


def plan_digest(plan: Mapping[str, object]) -> str:
    """The canonical plan digest, as `sha256:<hex>`."""
    return "sha256:" + hashlib.sha256(canonical_json(plan)).hexdigest()
