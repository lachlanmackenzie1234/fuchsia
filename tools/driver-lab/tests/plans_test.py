# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

"""Unit tests for plan validation, canonicalization, and digests."""

import unittest
from typing import Any

from driver_lab.plans import (
    PlanError,
    canonical_json,
    plan_digest,
    validate_plan,
)


def make_plan(**overrides: object) -> dict[str, Any]:
    plan: dict[str, Any] = {
        "schema_version": 1,
        "run_id": "2026-07-30T00-00-00Z-status-read",
        "case_id": "device-status-observation",
        "target": {"selector": "lab-target"},
        "node": {
            "id": "example-device",
            "expected_unclaimed": True,
            "expected_resource_digest": "sha256:" + "ab" * 32,
        },
        "access": {"mode": "proxy", "activation": "bind-unclaimed"},
        "operations": [
            {"kind": "mmio_read32", "resource": "control", "offset": "0x3c"},
        ],
    }
    plan.update(overrides)
    return plan


class CanonicalizationTest(unittest.TestCase):
    def test_digest_is_deterministic(self) -> None:
        self.assertEqual(plan_digest(make_plan()), plan_digest(make_plan()))
        self.assertTrue(plan_digest(make_plan()).startswith("sha256:"))

    def test_numeric_representation_is_normalized(self) -> None:
        hex_plan = make_plan()
        int_plan = make_plan(
            operations=[
                {"kind": "mmio_read32", "resource": "control", "offset": 0x3C}
            ]
        )
        self.assertEqual(plan_digest(hex_plan), plan_digest(int_plan))

    def test_key_order_is_normalized(self) -> None:
        reordered = make_plan()
        reordered["access"] = {"activation": "bind-unclaimed", "mode": "proxy"}
        self.assertEqual(plan_digest(make_plan()), plan_digest(reordered))

    def test_every_executable_change_invalidates(self) -> None:
        base = plan_digest(make_plan())
        changed = [
            make_plan(run_id="different-run"),
            make_plan(case_id="different-case"),
            make_plan(target={"selector": "other-target"}),
            make_plan(
                operations=[
                    {
                        "kind": "mmio_read32",
                        "resource": "control",
                        "offset": "0x40",
                    }
                ]
            ),
            make_plan(
                operations=[
                    {
                        "kind": "mmio_read32",
                        "resource": "status",
                        "offset": "0x3c",
                    }
                ]
            ),
            make_plan(access={"mode": "direct"}),
        ]
        for plan in changed:
            self.assertNotEqual(plan_digest(plan), base)

    def test_operation_order_is_semantic(self) -> None:
        forward = make_plan(
            operations=[
                {
                    "kind": "mmio_read32",
                    "resource": "control",
                    "offset": "0x3c",
                },
                {
                    "kind": "mmio_read32",
                    "resource": "control",
                    "offset": "0x40",
                },
            ]
        )
        reversed_ops = make_plan(
            operations=[
                {
                    "kind": "mmio_read32",
                    "resource": "control",
                    "offset": "0x40",
                },
                {
                    "kind": "mmio_read32",
                    "resource": "control",
                    "offset": "0x3c",
                },
            ]
        )
        self.assertNotEqual(plan_digest(forward), plan_digest(reversed_ops))

    def test_canonical_json_is_stable_bytes(self) -> None:
        self.assertEqual(
            canonical_json(make_plan()), canonical_json(make_plan())
        )


class ValidationTest(unittest.TestCase):
    def test_unknown_top_level_key_rejected(self) -> None:
        plan = make_plan()
        plan["driver_url"] = "fuchsia-pkg://evil"
        with self.assertRaises(PlanError):
            validate_plan(plan)

    def test_unknown_operation_key_rejected(self) -> None:
        plan = make_plan(
            operations=[
                {
                    "kind": "mmio_read32",
                    "resource": "control",
                    "offset": "0x3c",
                    "moniker": "core/evil",
                }
            ]
        )
        with self.assertRaises(PlanError):
            validate_plan(plan)

    def test_takeover_activation_rejected_until_phase_2(self) -> None:
        plan = make_plan(access={"mode": "proxy", "activation": "takeover"})
        with self.assertRaisesRegex(PlanError, "phase 2"):
            validate_plan(plan)

    def test_reserved_takeover_keys_rejected_as_reserved_not_unknown(
        self,
    ) -> None:
        # The schema carries the phase 2 keys: setting one fails with a
        # distinct reserved-key error so phase 2 adoption changes no
        # schema shape.
        for section, key in (
            ("node", "expected_bound_driver_url"),
            ("node", "expected_topology_generation"),
            ("access", "restoration"),
        ):
            plan = make_plan()
            plan[section] = dict(plan[section], **{key: "anything"})
            with self.assertRaisesRegex(
                PlanError, "reserved until phase 2", msg=key
            ):
                validate_plan(plan)

    def test_activation_invalid_in_direct_mode(self) -> None:
        plan = make_plan(
            access={"mode": "direct", "activation": "bind-unclaimed"}
        )
        with self.assertRaises(PlanError):
            validate_plan(plan)

    def test_write_operations_rejected(self) -> None:
        plan = make_plan(
            operations=[
                {
                    "kind": "mmio_write32",
                    "resource": "control",
                    "offset": "0x3c",
                }
            ]
        )
        with self.assertRaises(PlanError):
            validate_plan(plan)

    def test_empty_operations_rejected(self) -> None:
        with self.assertRaises(PlanError):
            validate_plan(make_plan(operations=[]))

    def test_bad_offset_rejected(self) -> None:
        plan = make_plan(
            operations=[
                {
                    "kind": "mmio_read32",
                    "resource": "control",
                    "offset": "0x3c; rm -rf",
                }
            ]
        )
        with self.assertRaises(PlanError):
            validate_plan(plan)

    def test_snapshot_operation_validates_items(self) -> None:
        good = make_plan(
            operations=[
                {
                    "kind": "mmio_snapshot32",
                    "items": [
                        {"resource": "control", "offset": "0x3c"},
                        {"resource": "control", "offset": 0x40},
                    ],
                }
            ]
        )
        validated = validate_plan(good)
        self.assertEqual(validated["operations"][0]["items"][0]["offset"], 0x3C)
        bad = make_plan(operations=[{"kind": "mmio_snapshot32", "items": []}])
        with self.assertRaises(PlanError):
            validate_plan(bad)

    def test_fidl_call_operation_validates(self) -> None:
        good = make_plan(
            access={"mode": "direct"},
            operations=[
                {
                    "kind": "fidl_call",
                    "method": "GetStatus",
                    "args": {"flags": 1},
                }
            ],
        )
        validated = validate_plan(good)
        self.assertEqual(validated["operations"][0]["kind"], "fidl_call")
        self.assertEqual(validated["operations"][0]["method"], "GetStatus")
        self.assertEqual(validated["operations"][0]["args"], {"flags": 1})

        bad_method = make_plan(operations=[{"kind": "fidl_call", "method": ""}])
        with self.assertRaises(PlanError):
            validate_plan(bad_method)

        bad_args = make_plan(
            operations=[
                {"kind": "fidl_call", "method": "Foo", "args": "not-a-map"}
            ]
        )
        with self.assertRaises(PlanError):
            validate_plan(bad_args)


if __name__ == "__main__":
    unittest.main()
