# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

"""End-to-end tests for the plan-driven API over the fake proxy target."""

import json
import tempfile
import unittest
from pathlib import Path
from typing import Any

from driver_lab.api import (
    EXIT_ACTIVATION,
    EXIT_OPERATION,
    EXIT_PERMISSION,
    EXIT_STALE,
    EXIT_TRANSPORT,
    EXIT_UNSUPPORTED,
    DriverLab,
)
from driver_lab.discovery import FakeNodeDiscovery, NodeDescription
from driver_lab.evidence import EvidenceError
from driver_lab.models import AccessClass, Decision, ReadGrant
from driver_lab.permissions import save_grants
from driver_lab.plans import PlanError, validate_plan
from driver_lab.transport import (
    FakeDirectTarget,
    FakeProxyTarget,
    OpenRejection,
    ProxyDescription,
    ResourceInfo,
)

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


class RunPlanTest(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        self._dir = tempfile.TemporaryDirectory()
        self.addCleanup(self._dir.cleanup)
        base = Path(self._dir.name)
        self.grants_path = base / "grants.toml"
        self.evidence_root = base / "evidence"
        self.fake = FakeProxyTarget(make_description())
        self.fake.set_value(1, 0x3C, 0xDEAD_BEEF)
        self.fake.set_value(1, 0x40, 0x1234_5678)
        self.lab = DriverLab(
            self.fake,
            grants_path=self.grants_path,
            evidence_root=self.evidence_root,
            target_scope=TARGET_SCOPE,
            node_id=NODE_ID,
        )

    def read_manifest(self, evidence_dir: Path) -> dict[str, Any]:
        manifest = json.loads((evidence_dir / "manifest.json").read_text())
        assert isinstance(manifest, dict)
        return manifest

    async def test_happy_read_produces_complete_evidence(self) -> None:
        save_grants(self.grants_path, [make_grant()])
        result = await self.lab.run_plan(make_plan())
        self.assertTrue(result.ok, result.failure)
        self.assertEqual(result.reads[0].value, 0xDEAD_BEEF)
        self.assertEqual(self.fake.sessions_opened, 1)

        manifest = self.read_manifest(result.evidence_dir)
        for name in (
            "plan.requested.json",
            "plan.canonical.json",
            "access.capabilities.json",
            "target.description.json",
            "permission-resolution.json",
            "operations.jsonl",
            "target-audit.jsonl",
        ):
            self.assertIsInstance(manifest["files"][name], dict, name)
        # Artifacts a phase 1 read-only run cannot produce are marked
        # not applicable, never silently omitted.
        for name in (
            "approval.json",
            "restoration.json",
            "proxy-access.jsonl",
            "serial.log",
        ):
            self.assertEqual(manifest["files"][name], "not_applicable", name)
        self.assertEqual(manifest["plan_digest"], result.plan_digest)

        audit_ops = [
            json.loads(line)["operation"]
            for line in (result.evidence_dir / "target-audit.jsonl")
            .read_text()
            .splitlines()
        ]
        self.assertIn("open_session", audit_ops)
        self.assertIn("read32", audit_ops)
        # The session-closed entry is appended after the final drain (audit
        # is drained through the session, then the session closes), so it
        # is not part of this run's evidence.
        self.assertNotIn("session_closed", audit_ops)

    async def test_missing_grant_fails_closed_before_any_target_access(
        self,
    ) -> None:
        result = await self.lab.run_plan(make_plan())
        self.assertEqual(result.exit_category, EXIT_PERMISSION)
        self.assertEqual(self.fake.open_attempts, 0)
        manifest = self.read_manifest(result.evidence_dir)
        self.assertEqual(
            manifest["files"]["operations.jsonl"], "not_applicable"
        )
        self.assertIsInstance(
            manifest["files"]["permission-resolution.json"], dict
        )

    async def test_persistent_deny_wins(self) -> None:
        save_grants(self.grants_path, [make_grant(decision=Decision.DENY)])
        result = await self.lab.run_plan(make_plan())
        self.assertEqual(result.exit_category, EXIT_PERMISSION)
        self.assertEqual(result.failure, "denied by persistent grant")
        self.assertEqual(self.fake.open_attempts, 0)

    async def test_read_once_grant_does_not_cover_snapshot(self) -> None:
        save_grants(self.grants_path, [make_grant()])
        plan = make_plan(
            operations=[
                {
                    "kind": "mmio_snapshot32",
                    "items": [{"resource": "control", "offset": "0x3c"}],
                }
            ]
        )
        result = await self.lab.run_plan(plan)
        self.assertEqual(result.exit_category, EXIT_PERMISSION)
        self.assertEqual(self.fake.open_attempts, 0)

    async def test_stale_resource_digest_fails_before_session(self) -> None:
        save_grants(self.grants_path, [make_grant()])
        plan = make_plan()
        plan["node"] = dict(
            plan["node"], expected_resource_digest="sha256:" + "cd" * 32
        )
        result = await self.lab.run_plan(plan)
        self.assertEqual(result.exit_category, EXIT_STALE)
        self.assertEqual(self.fake.open_attempts, 0)

    async def test_unknown_resource_is_stale(self) -> None:
        save_grants(self.grants_path, [make_grant()])
        plan = make_plan(
            operations=[
                {"kind": "mmio_read32", "resource": "missing", "offset": "0x0"}
            ]
        )
        result = await self.lab.run_plan(plan)
        self.assertEqual(result.exit_category, EXIT_STALE)
        self.assertEqual(self.fake.open_attempts, 0)

    async def test_session_rejection_maps_stale_and_activation(self) -> None:
        save_grants(self.grants_path, [make_grant()])
        self.fake.reject_open = OpenRejection.STALE_PROXY_GENERATION
        result = await self.lab.run_plan(make_plan())
        self.assertEqual(result.exit_category, EXIT_STALE)
        self.assertEqual(self.fake.sessions_opened, 0)

        self.fake.reject_open = OpenRejection.MUTATION_LEASE_HELD
        result = await self.lab.run_plan(make_plan(run_id="run-2"))
        self.assertEqual(result.exit_category, EXIT_ACTIVATION)

    async def test_backend_fault_still_drains_audit(self) -> None:
        save_grants(self.grants_path, [make_grant()])
        self.fake.fail_at(1, 0x3C)
        result = await self.lab.run_plan(make_plan())
        self.assertEqual(result.exit_category, EXIT_OPERATION)
        self.assertEqual(result.failure, "backend_fault")
        operations = [
            json.loads(line)
            for line in (result.evidence_dir / "operations.jsonl")
            .read_text()
            .splitlines()
        ]
        self.assertEqual(operations[0]["error"], "backend_fault")
        audit_statuses = [
            json.loads(line)["status"]
            for line in (result.evidence_dir / "target-audit.jsonl")
            .read_text()
            .splitlines()
        ]
        self.assertIn("backend_fault", audit_statuses)

    async def test_snapshot_happy_path(self) -> None:
        save_grants(
            self.grants_path,
            [
                make_grant(access=AccessClass.SNAPSHOT),
                make_grant(access=AccessClass.SNAPSHOT, offset=0x40),
            ],
        )
        plan = make_plan(
            operations=[
                {
                    "kind": "mmio_snapshot32",
                    "items": [
                        {"resource": "control", "offset": "0x3c"},
                        {"resource": "control", "offset": "0x40"},
                    ],
                }
            ]
        )
        result = await self.lab.run_plan(plan)
        self.assertTrue(result.ok, result.failure)
        [row] = [
            json.loads(line)
            for line in (result.evidence_dir / "operations.jsonl")
            .read_text()
            .splitlines()
        ]
        self.assertTrue(row["complete"])
        self.assertEqual(
            [item["value"] for item in row["results"]],
            [0xDEAD_BEEF, 0x1234_5678],
        )

    async def test_evidence_directory_is_never_reused(self) -> None:
        save_grants(self.grants_path, [make_grant()])
        await self.lab.run_plan(make_plan())
        with self.assertRaises(EvidenceError):
            await self.lab.run_plan(make_plan())

    async def test_direct_mode_is_unsupported_not_substituted(self) -> None:
        save_grants(self.grants_path, [make_grant()])
        result = await self.lab.run_plan(make_plan(access={"mode": "direct"}))
        self.assertEqual(result.exit_category, EXIT_UNSUPPORTED)
        # A direct-mode plan must never silently execute over the proxy.
        self.assertEqual(self.fake.open_attempts, 0)

    async def test_direct_mode_happy_path(self) -> None:
        fake_direct = FakeDirectTarget(
            node_id=NODE_ID,
            protocol_name="fuchsia.hardware.example/Device",
            boot_id="boot-1",
        )
        fake_direct.register_method(
            "GetStatus", lambda args: {"status": "ok", "code": 0}
        )
        direct_lab = DriverLab(
            fake_direct,
            grants_path=self.grants_path,
            evidence_root=self.evidence_root,
            target_scope=TARGET_SCOPE,
            node_id=NODE_ID,
        )
        plan = {
            "schema_version": 1,
            "run_id": "direct-run-1",
            "case_id": "direct-status-case",
            "target": {"selector": "lab-target", "expected_boot_id": "boot-1"},
            "node": {"id": NODE_ID},
            "access": {"mode": "direct"},
            "operations": [
                {"kind": "fidl_call", "method": "GetStatus", "args": {}}
            ],
        }
        result = await direct_lab.run_plan(plan)
        self.assertTrue(result.ok, result.failure)
        self.assertEqual(len(result.calls), 1)
        self.assertEqual(result.calls[0]["method"], "GetStatus")
        self.assertEqual(
            result.calls[0]["response"], {"status": "ok", "code": 0}
        )
        self.assertEqual(fake_direct.sessions_opened, 1)

        manifest = self.read_manifest(result.evidence_dir)
        self.assertEqual(
            manifest["files"]["target-audit.jsonl"], "not_applicable"
        )
        self.assertEqual(
            manifest["files"]["permission-resolution.json"], "not_applicable"
        )

        capabilities = json.loads(
            (result.evidence_dir / "access.capabilities.json").read_text()
        )
        self.assertEqual(capabilities["mode"], "direct")
        self.assertFalse(capabilities["target_policy"])
        self.assertFalse(capabilities["target_audit"])

        target_desc = json.loads(
            (result.evidence_dir / "target.description.json").read_text()
        )
        self.assertEqual(target_desc["node_id"], NODE_ID)
        self.assertEqual(
            target_desc["protocol_name"], "fuchsia.hardware.example/Device"
        )

        ops_lines = (
            (result.evidence_dir / "operations.jsonl").read_text().splitlines()
        )
        self.assertEqual(len(ops_lines), 1)
        row = json.loads(ops_lines[0])
        self.assertEqual(row["method"], "GetStatus")
        self.assertEqual(row["response"], {"status": "ok", "code": 0})

    async def test_direct_mode_rejects_private_mmio(self) -> None:
        fake_direct = FakeDirectTarget()
        direct_lab = DriverLab(
            fake_direct,
            grants_path=self.grants_path,
            evidence_root=self.evidence_root,
            target_scope=TARGET_SCOPE,
            node_id=NODE_ID,
        )
        plan = {
            "schema_version": 1,
            "run_id": "direct-run-mmio",
            "case_id": "case-mmio",
            "target": {"selector": "lab-target"},
            "node": {"id": NODE_ID},
            "access": {"mode": "direct"},
            "operations": [
                {"kind": "mmio_read32", "resource": "control", "offset": 0x3C}
            ],
        }
        result = await direct_lab.run_plan(plan)
        self.assertEqual(result.exit_category, EXIT_UNSUPPORTED)
        self.assertEqual(
            result.failure, "direct mode does not provide private MMIO access"
        )
        self.assertEqual(fake_direct.sessions_opened, 0)

    async def test_direct_mode_rejects_target_audit_requirement(self) -> None:
        fake_direct = FakeDirectTarget()
        direct_lab = DriverLab(
            fake_direct,
            grants_path=self.grants_path,
            evidence_root=self.evidence_root,
            target_scope=TARGET_SCOPE,
            node_id=NODE_ID,
        )
        plan = {
            "schema_version": 1,
            "run_id": "direct-run-audit",
            "case_id": "case-audit",
            "target": {"selector": "lab-target"},
            "node": {"id": NODE_ID},
            "access": {"mode": "direct", "requires_target_audit": True},
            "operations": [{"kind": "fidl_call", "method": "GetStatus"}],
        }
        result = await direct_lab.run_plan(plan)
        self.assertEqual(result.exit_category, EXIT_UNSUPPORTED)
        self.assertIn("target_audit", str(result.failure))
        self.assertEqual(fake_direct.sessions_opened, 0)

    async def test_proxy_mode_rejects_direct_transport(self) -> None:
        fake_direct = FakeDirectTarget()
        direct_lab = DriverLab(
            fake_direct,
            grants_path=self.grants_path,
            evidence_root=self.evidence_root,
            target_scope=TARGET_SCOPE,
            node_id=NODE_ID,
        )
        plan = make_plan()
        result = await direct_lab.run_plan(plan)
        self.assertEqual(result.exit_category, EXIT_UNSUPPORTED)
        self.assertEqual(
            result.failure, "transport does not support proxy mode"
        )

    async def test_direct_mode_no_fallback_on_connection_failure(self) -> None:
        fake_direct = FakeDirectTarget()
        fake_direct.fail_open = True
        direct_lab = DriverLab(
            fake_direct,
            grants_path=self.grants_path,
            evidence_root=self.evidence_root,
            target_scope=TARGET_SCOPE,
            node_id=NODE_ID,
        )
        plan = {
            "schema_version": 1,
            "run_id": "direct-fail-open",
            "case_id": "case-fail",
            "target": {"selector": "lab-target"},
            "node": {"id": NODE_ID},
            "access": {"mode": "direct"},
            "operations": [{"kind": "fidl_call", "method": "GetStatus"}],
        }
        result = await direct_lab.run_plan(plan)
        self.assertEqual(result.exit_category, EXIT_TRANSPORT)
        self.assertIn("direct connection failed", str(result.failure))
        self.assertEqual(fake_direct.sessions_opened, 0)

    def test_direct_mode_refuses_takeover_activation(self) -> None:
        plan = {
            "schema_version": 1,
            "run_id": "direct-takeover",
            "case_id": "case-takeover",
            "target": {"selector": "lab-target"},
            "node": {"id": NODE_ID},
            "access": {"mode": "direct", "activation": "takeover"},
            "operations": [{"kind": "fidl_call", "method": "GetStatus"}],
        }
        with self.assertRaises(PlanError):
            validate_plan(plan)

    async def test_typed_protocol_discovery(self) -> None:
        discovery = FakeNodeDiscovery(
            [
                NodeDescription(
                    moniker="node-1",
                    offers=("fuchsia.hardware.gpio/Device",),
                ),
                NodeDescription(
                    moniker="node-2",
                    offers=("fuchsia.hardware.i2c/Device",),
                ),
            ]
        )
        gpio_nodes = await discovery.find_nodes_offering(
            "fuchsia.hardware.gpio/Device"
        )
        self.assertEqual(len(gpio_nodes), 1)
        self.assertEqual(gpio_nodes[0].moniker, "node-1")
        self.assertEqual(
            gpio_nodes[0].offers, ("fuchsia.hardware.gpio/Device",)
        )

    async def test_required_guarantee_fails_before_connection(self) -> None:
        save_grants(self.grants_path, [make_grant()])
        plan = make_plan(
            access={
                "mode": "proxy",
                "activation": "bind-unclaimed",
                "requires_target_local_timing": True,
            }
        )
        result = await self.lab.run_plan(plan)
        self.assertEqual(result.exit_category, EXIT_UNSUPPORTED)
        self.assertEqual(self.fake.open_attempts, 0)

    async def test_expected_unclaimed_is_unverifiable_and_fails_closed(
        self,
    ) -> None:
        save_grants(self.grants_path, [make_grant()])
        plan = make_plan()
        plan["node"] = dict(plan["node"], expected_unclaimed=True)
        result = await self.lab.run_plan(plan)
        self.assertEqual(result.exit_category, EXIT_UNSUPPORTED)
        self.assertEqual(self.fake.open_attempts, 0)

    async def test_expected_unclaimed_verified_with_discovery(self) -> None:
        save_grants(self.grants_path, [make_grant()])
        discovery = FakeNodeDiscovery(
            [
                NodeDescription(
                    moniker=NODE_ID,
                    bound_driver_url=None,
                )
            ]
        )
        lab = DriverLab(
            self.fake,
            grants_path=self.grants_path,
            evidence_root=self.evidence_root,
            target_scope=TARGET_SCOPE,
            node_id=NODE_ID,
            discovery=discovery,
        )
        plan = make_plan()
        plan["node"] = dict(plan["node"], expected_unclaimed=True)
        result = await lab.run_plan(plan)
        self.assertTrue(result.ok, result.failure)
        self.assertEqual(self.fake.sessions_opened, 1)

    async def test_expected_unclaimed_fails_when_bound(self) -> None:
        save_grants(self.grants_path, [make_grant()])
        discovery = FakeNodeDiscovery(
            [
                NodeDescription(
                    moniker=NODE_ID,
                    bound_driver_url="fuchsia-boot:///driver#meta/driver.cm",
                )
            ]
        )
        lab = DriverLab(
            self.fake,
            grants_path=self.grants_path,
            evidence_root=self.evidence_root,
            target_scope=TARGET_SCOPE,
            node_id=NODE_ID,
            discovery=discovery,
        )
        plan = make_plan()
        plan["node"] = dict(plan["node"], expected_unclaimed=True)
        result = await lab.run_plan(plan)
        self.assertEqual(result.exit_category, EXIT_ACTIVATION)
        self.assertIsNotNone(result.failure)
        assert result.failure is not None
        self.assertIn("managed takeover is phase 2", result.failure)
        self.assertEqual(self.fake.open_attempts, 0)

    async def test_expected_bound_fails_when_unclaimed(self) -> None:
        save_grants(self.grants_path, [make_grant()])
        discovery = FakeNodeDiscovery(
            [
                NodeDescription(
                    moniker=NODE_ID,
                    bound_driver_url=None,
                )
            ]
        )
        lab = DriverLab(
            self.fake,
            grants_path=self.grants_path,
            evidence_root=self.evidence_root,
            target_scope=TARGET_SCOPE,
            node_id=NODE_ID,
            discovery=discovery,
        )
        plan = make_plan()
        plan["node"] = dict(plan["node"], expected_unclaimed=False)
        result = await lab.run_plan(plan)
        self.assertEqual(result.exit_category, EXIT_ACTIVATION)
        self.assertIsNotNone(result.failure)
        assert result.failure is not None
        self.assertIn("is unclaimed, expected bound driver", result.failure)
        self.assertEqual(self.fake.open_attempts, 0)

    async def test_expected_unclaimed_fails_when_node_not_found(self) -> None:
        save_grants(self.grants_path, [make_grant()])
        discovery = FakeNodeDiscovery([])
        lab = DriverLab(
            self.fake,
            grants_path=self.grants_path,
            evidence_root=self.evidence_root,
            target_scope=TARGET_SCOPE,
            node_id=NODE_ID,
            discovery=discovery,
        )
        plan = make_plan()
        plan["node"] = dict(plan["node"], expected_unclaimed=True)
        result = await lab.run_plan(plan)
        self.assertEqual(result.exit_category, EXIT_ACTIVATION)
        self.assertIsNotNone(result.failure)
        assert result.failure is not None
        self.assertIn("not found during discovery", result.failure)
        self.assertEqual(self.fake.open_attempts, 0)

    async def test_list_nodes_and_describe_node(self) -> None:
        discovery = FakeNodeDiscovery(
            [
                NodeDescription(
                    moniker=NODE_ID,
                    bound_driver_url=None,
                )
            ]
        )
        lab = DriverLab(
            self.fake,
            grants_path=self.grants_path,
            evidence_root=self.evidence_root,
            target_scope=TARGET_SCOPE,
            node_id=NODE_ID,
            discovery=discovery,
        )
        nodes = await lab.list_nodes()
        self.assertEqual(len(nodes), 1)
        self.assertEqual(nodes[0].moniker, NODE_ID)
        desc = await lab.describe_node(NODE_ID)
        self.assertIsNotNone(desc)
        assert desc is not None
        self.assertEqual(desc.moniker, NODE_ID)

    async def test_expected_boot_id_is_enforced(self) -> None:
        save_grants(self.grants_path, [make_grant()])
        plan = make_plan()
        plan["target"] = dict(plan["target"], expected_boot_id="boot-2")
        result = await self.lab.run_plan(plan)
        self.assertEqual(result.exit_category, EXIT_STALE)
        self.assertEqual(
            result.failure, "expected_boot_id does not match target"
        )
        self.assertEqual(self.fake.open_attempts, 0)

        plan = make_plan(run_id="run-2")
        plan["target"] = dict(plan["target"], expected_boot_id="boot-1")
        result = await self.lab.run_plan(plan)
        self.assertTrue(result.ok, result.failure)

    async def test_audit_entries_carry_run_and_instance_identity(self) -> None:
        save_grants(self.grants_path, [make_grant()])
        result = await self.lab.run_plan(make_plan())
        self.assertTrue(result.ok, result.failure)
        rows = [
            json.loads(line)
            for line in (result.evidence_dir / "target-audit.jsonl")
            .read_text()
            .splitlines()
        ]
        opens = [row for row in rows if row["operation"] == "open_session"]
        self.assertEqual(opens[0]["run_id"], "run-1")
        for row in rows:
            self.assertEqual(row["boot_id"], "boot-1")
            self.assertEqual(row["proxy_generation"], 7)


if __name__ == "__main__":
    unittest.main()
