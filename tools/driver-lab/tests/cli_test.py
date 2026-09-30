# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

"""Tests for the driver-lab CLI."""

import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path
from typing import Any
from unittest import mock

from driver_lab import cli
from driver_lab.models import AccessClass, Decision, ReadGrant
from driver_lab.permissions import load_grants, save_grants
from driver_lab.plans import plan_digest
from driver_lab.transport import FakeProxyTarget, ProxyDescription, ResourceInfo

DIGEST = "sha256:" + "ab" * 32


def make_grant(**overrides: object) -> ReadGrant:
    fields: dict[str, Any] = dict(
        schema_version=1,
        target_scope="scope",
        node_id="node",
        resource_digest=DIGEST,
        resource="control",
        offset=0x3C,
        width=4,
        access=AccessClass.READ_ONCE,
        decision=Decision.ALLOW,
        approved_at="2026-07-30T00:00:00Z",
    )
    fields.update(overrides)
    return ReadGrant(**fields)


def make_plan() -> dict[str, Any]:
    return {
        "schema_version": 1,
        "run_id": "run-1",
        "case_id": "case-1",
        "target": {"selector": "lab-target"},
        "node": {"id": "node"},
        "access": {"mode": "proxy", "activation": "bind-unclaimed"},
        "operations": [
            {"kind": "mmio_read32", "resource": "control", "offset": "0x3c"},
        ],
    }


class CliTest(unittest.TestCase):
    def setUp(self) -> None:
        self._dir = tempfile.TemporaryDirectory()
        self.addCleanup(self._dir.cleanup)
        self.base = Path(self._dir.name)
        self.grants = self.base / "grants.toml"

    def run_cli(self, *argv: str) -> tuple[int, dict[str, Any], str]:
        stdout = io.StringIO()
        stderr = io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(
            stderr
        ):
            code = cli.main(list(argv))
        payload = json.loads(stdout.getvalue()) if stdout.getvalue() else {}
        return code, payload, stderr.getvalue()

    def test_permissions_list(self) -> None:
        save_grants(self.grants, [make_grant()])
        code, payload, _ = self.run_cli(
            "permissions", "list", "--grants", str(self.grants)
        )
        self.assertEqual(code, 0)
        [row] = payload["grants"]
        self.assertEqual(row["offset"], 0x3C)
        self.assertTrue(row["grant_id"].startswith("grant-"))

    def test_permissions_revoke(self) -> None:
        save_grants(self.grants, [make_grant()])
        grant_id = make_grant().grant_id
        code, payload, _ = self.run_cli(
            "permissions",
            "revoke",
            "--grants",
            str(self.grants),
            "--grant-id",
            grant_id,
        )
        self.assertEqual(code, 0)
        self.assertEqual(payload["revoked"], grant_id)
        self.assertEqual(load_grants(self.grants), [])

        code, _, stderr = self.run_cli(
            "permissions",
            "revoke",
            "--grants",
            str(self.grants),
            "--grant-id",
            grant_id,
        )
        self.assertEqual(code, 2)
        self.assertIn("no grant", stderr)

    def test_permissions_explain(self) -> None:
        save_grants(self.grants, [make_grant()])
        code, payload, _ = self.run_cli(
            "permissions",
            "explain",
            "--grants",
            str(self.grants),
            "--target-scope",
            "scope",
            "--node-id",
            "node",
            "--resource-digest",
            DIGEST,
            "--resource",
            "control",
            "--offset",
            "0x3c",
        )
        self.assertEqual(code, 0)
        self.assertEqual(payload["outcome"], "allowed")

        code, payload, _ = self.run_cli(
            "permissions",
            "explain",
            "--grants",
            str(self.grants),
            "--target-scope",
            "scope",
            "--node-id",
            "node",
            "--resource-digest",
            DIGEST,
            "--resource",
            "control",
            "--offset",
            "0x40",
        )
        self.assertEqual(code, 0)
        self.assertEqual(payload["outcome"], "undecided")

    def test_plan_digest_matches_library(self) -> None:
        plan_path = self.base / "plan.json"
        plan_path.write_text(json.dumps(make_plan()))
        code, payload, _ = self.run_cli(
            "plan", "digest", "--plan", str(plan_path)
        )
        self.assertEqual(code, 0)
        self.assertEqual(payload["digest"], plan_digest(make_plan()))

    def test_plan_validate_rejects_takeover(self) -> None:
        plan = make_plan()
        plan["access"] = {"mode": "proxy", "activation": "takeover"}
        plan_path = self.base / "plan.json"
        plan_path.write_text(json.dumps(plan))
        code, _, stderr = self.run_cli(
            "plan", "validate", "--plan", str(plan_path)
        )
        self.assertEqual(code, 2)
        self.assertIn("phase 2", stderr)

    def test_malformed_grants_file_is_an_error(self) -> None:
        self.grants.write_text("not toml [")
        code, _, stderr = self.run_cli(
            "permissions", "list", "--grants", str(self.grants)
        )
        self.assertEqual(code, 2)
        self.assertIn("error", stderr)

    def test_permissions_add_persists_and_prints_exact_rule(self) -> None:
        code, payload, _ = self.run_cli(
            "permissions",
            "add",
            "--grants",
            str(self.grants),
            "--target-scope",
            "scope",
            "--node-id",
            "node",
            "--resource-digest",
            DIGEST,
            "--resource",
            "control",
            "--offset",
            "0x3c",
            "--decision",
            "allow",
        )
        self.assertEqual(code, 0)
        self.assertEqual(payload["added"]["offset"], 0x3C)
        self.assertEqual(payload["added"]["decision"], "allow")
        self.assertEqual(payload["added"]["approval_source"], "cli")
        [stored] = load_grants(self.grants)
        self.assertEqual(stored.grant_id, payload["added"]["grant_id"])

        # A duplicate exact rule is rejected, not silently merged.
        code, _, stderr = self.run_cli(
            "permissions",
            "add",
            "--grants",
            str(self.grants),
            "--target-scope",
            "scope",
            "--node-id",
            "node",
            "--resource-digest",
            DIGEST,
            "--resource",
            "control",
            "--offset",
            "0x3c",
            "--decision",
            "deny",
        )
        self.assertEqual(code, 2)
        self.assertIn("error", stderr)

    def _run_fixture(self) -> FakeProxyTarget:
        description = ProxyDescription(
            protocol_major=1,
            protocol_minor=0,
            proxy_generation=7,
            boot_id="boot-1",
            resource_digest="sha256:" + "ee" * 32,
            policy_digest="sha256:" + "ff" * 32,
            resources=(
                ResourceInfo(
                    id=1, name="control", logical_size=0x100, digest=DIGEST
                ),
            ),
            max_snapshot_items=64,
            audit_capacity=1024,
        )
        fake = FakeProxyTarget(description)
        fake.set_value(1, 0x3C, 0xDEAD_BEEF)
        return fake

    def run_cli_run(
        self, fake: FakeProxyTarget, run_id: str
    ) -> tuple[int, dict[str, Any], str]:
        plan = make_plan()
        plan["run_id"] = run_id
        plan_path = self.base / f"{run_id}.json"
        plan_path.write_text(json.dumps(plan))
        with mock.patch.object(cli, "_connect_transport", return_value=fake):
            return self.run_cli(
                "run",
                "--plan",
                str(plan_path),
                "--evidence-dir",
                str(self.base / "evidence"),
                "--grants",
                str(self.grants),
                "--target-scope",
                "scope",
                "--node-id",
                "node",
                "--moniker",
                "bootstrap/driver-lab",
            )

    def test_run_executes_plan_to_finalized_evidence(self) -> None:
        save_grants(self.grants, [make_grant()])
        fake = self._run_fixture()
        code, payload, _ = self.run_cli_run(fake, "run-1")
        self.assertEqual(code, 0)
        self.assertEqual(payload["reads"][0]["value"], 0xDEAD_BEEF)
        manifest_path = Path(payload["evidence_dir"]) / "manifest.json"
        self.assertTrue(manifest_path.exists())

    def test_run_unattended_fails_closed(self) -> None:
        fake = self._run_fixture()
        code, payload, _ = self.run_cli_run(fake, "run-1")
        self.assertEqual(code, 2)
        self.assertIn("fails closed", payload["failure"])
        self.assertEqual(fake.open_attempts, 0)

    def test_run_evidence_reuse_maps_to_evidence_category(self) -> None:
        save_grants(self.grants, [make_grant()])
        fake = self._run_fixture()
        code, _, _ = self.run_cli_run(fake, "run-1")
        self.assertEqual(code, 0)
        code, _, stderr = self.run_cli_run(fake, "run-1")
        self.assertEqual(code, 7)
        self.assertIn("no reuse", stderr)


if __name__ == "__main__":
    unittest.main()
