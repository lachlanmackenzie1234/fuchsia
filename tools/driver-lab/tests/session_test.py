# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

import asyncio
import tempfile
import unittest
from pathlib import Path
from typing import Any

from driver_lab.api import DriverLab, DriverLabError
from driver_lab.consent import ConsentDecision
from driver_lab.models import AccessRequest
from driver_lab.session import (
    AccessRequirements,
    HardwareSession,
    MmioRegion,
    SessionCapabilities,
    TranslationMetadata,
    UnsupportedCapabilityError,
)
from driver_lab.transport import (
    Denial,
    DirectDescription,
    FakeDirectTarget,
    FakeProxyTarget,
    OperationDenied,
    ProxyDescription,
    ResourceInfo,
    SessionMode,
)

CTRL_DIGEST = "sha256:" + "11" * 32
POLICY_DIGEST = "sha256:" + "22" * 32


class ScriptedPrompt:
    def __init__(self, decision: ConsentDecision) -> None:
        self.decision = decision

    async def request_consent(
        self, request: AccessRequest, warning: str
    ) -> ConsentDecision:
        return self.decision


def make_proxy_description() -> ProxyDescription:
    return ProxyDescription(
        protocol_major=1,
        protocol_minor=0,
        proxy_generation=1,
        boot_id="boot-test-1",
        resource_digest="sha256:" + "33" * 32,
        policy_digest=POLICY_DIGEST,
        resources=(
            ResourceInfo(
                id=1,
                name="control",
                logical_size=0x100,
                digest=CTRL_DIGEST,
            ),
            ResourceInfo(
                id=2,
                name="status",
                logical_size=0x80,
                digest="sha256:" + "44" * 32,
            ),
        ),
        max_snapshot_items=64,
        audit_capacity=256,
        node_moniker="sample/hardware/node",
    )


class SessionTest(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        self._dir = tempfile.TemporaryDirectory()
        self.addCleanup(self._dir.cleanup)
        base = Path(self._dir.name)
        self.grants_path = base / "grants.toml"
        self.evidence_root = base / "evidence"

        self.fake_proxy = FakeProxyTarget(make_proxy_description())
        self.fake_direct = FakeDirectTarget(
            node_id="sample-node",
            protocol_name="fuchsia.hardware.gpio/Device",
            bound_driver_url="fuchsia-boot:///sample#meta/sample.cm",
        )

    async def test_proxy_mmio_read_and_write(self) -> None:
        self.fake_proxy.set_value(1, 0x10, 0xCAFE_BABE)
        lab = DriverLab(
            self.fake_proxy,
            grants_path=self.grants_path,
            evidence_root=self.evidence_root,
            target_scope="target-1",
            node_id="node-1",
        )

        prompt = ScriptedPrompt(ConsentDecision.ALLOW_ONCE)
        async with await lab.attach(
            "node-1",
            mode="proxy",
            session_mode=SessionMode.MUTATING,
            consent=prompt,
        ) as session:
            self.assertEqual(session.mode, "proxy")
            self.assertTrue(session.capabilities.target_policy)
            self.assertTrue(session.capabilities.target_audit)
            self.assertTrue(session.capabilities.target_local_timing)
            self.assertFalse(session.capabilities.production_driver_active)

            mmio = await session.mmio("control")
            self.assertEqual(mmio.name, "control")
            self.assertEqual(mmio.id, 1)
            self.assertEqual(mmio.logical_size, 0x100)

            # Read32
            val = await mmio.read32(0x10)
            self.assertEqual(val, 0xCAFE_BABE)

            # Write32 with readback
            write_res = await mmio.write32(0x14, 0x1234_5678)
            self.assertEqual(write_res.readback_value, 0x1234_5678)
            self.assertEqual(self.fake_proxy.get_value(1, 0x14), 0x1234_5678)

            # Write32 with precondition
            write_res2 = await mmio.write32(
                0x14, 0x8765_4321, expected_before=0x1234_5678
            )
            self.assertEqual(write_res2.readback_value, 0x8765_4321)
            self.assertEqual(self.fake_proxy.get_value(1, 0x14), 0x8765_4321)

            # Precondition failure
            with self.assertRaises(OperationDenied) as ctx:
                await mmio.write32(
                    0x14, 0xAAAA_BBBB, expected_before=0xDEAD_BEEF
                )
            self.assertEqual(ctx.exception.denial, Denial.PRECONDITION_FAILED)

    async def test_proxy_mmio_poll_and_snapshot(self) -> None:
        self.fake_proxy.set_value(1, 0x20, 0x0)
        lab = DriverLab(
            self.fake_proxy,
            grants_path=self.grants_path,
            evidence_root=self.evidence_root,
            target_scope="target-1",
            node_id="node-1",
        )

        async with await lab.attach("node-1", mode="proxy") as session:
            mmio = await session.mmio("control")

            # Poll matching value
            self.fake_proxy.set_value(1, 0x20, 0x1)
            poll_res = await mmio.poll32(0x20, expected=0x1, mask=0x1)
            self.assertEqual(poll_res.value, 0x1)

            # Poll timeout
            self.fake_proxy.set_value(1, 0x24, 0x0)
            with self.assertRaises(OperationDenied) as ctx:
                await mmio.poll32(0x24, expected=0x1, mask=0x1, timeout_s=0.001)
            self.assertEqual(ctx.exception.denial, Denial.TIMEOUT)

            # Snapshot32
            self.fake_proxy.set_value(1, 0x0, 0x1111)
            self.fake_proxy.set_value(1, 0x4, 0x2222)
            values = await mmio.snapshot32([0x0, 0x4])
            self.assertEqual(values, [0x1111, 0x2222])

    async def test_mmio_bounds_and_alignment_validation(self) -> None:
        lab = DriverLab(
            self.fake_proxy,
            grants_path=self.grants_path,
            evidence_root=self.evidence_root,
            target_scope="target-1",
            node_id="node-1",
        )
        async with await lab.attach("node-1", mode="proxy") as session:
            mmio = await session.mmio("control")

            with self.assertRaises(ValueError):
                await mmio.read32(-4)

            with self.assertRaises(ValueError):
                await mmio.read32(
                    0x100
                )  # Out of bounds (logical_size is 0x100)

            with self.assertRaises(ValueError):
                await mmio.read32(0x2)  # Misaligned

            with self.assertRaises(ValueError):
                await mmio.write32(0x100, 0x1)

            with self.assertRaises(ValueError):
                await mmio.poll32(0x1, expected=0)

            with self.assertRaises(ValueError):
                await session.mmio("nonexistent_resource")

    async def test_translation_metadata_present(self) -> None:
        self.assertIsInstance(MmioRegion.read32_metadata, TranslationMetadata)
        self.assertTrue(MmioRegion.read32_metadata.directly_translatable)
        self.assertIn("Read32", MmioRegion.read32_metadata.cpp_analogue)

        self.assertIsInstance(MmioRegion.write32_metadata, TranslationMetadata)
        self.assertTrue(MmioRegion.write32_metadata.directly_translatable)

        self.assertIsInstance(MmioRegion.poll32_metadata, TranslationMetadata)
        self.assertTrue(MmioRegion.poll32_metadata.target_local_timing)

        self.assertIsInstance(
            MmioRegion.snapshot32_metadata, TranslationMetadata
        )
        self.assertTrue(MmioRegion.snapshot32_metadata.experiment_only)

    async def test_direct_mode_capabilities_and_protocols(self) -> None:
        # Register handlers on fake direct target
        gpio_state = {"value": True, "direction": "output"}

        def handle_read(args: Any) -> dict[str, Any]:
            return {"value": gpio_state["value"]}

        def handle_write(args: Any) -> dict[str, Any]:
            gpio_state["value"] = args.get("value", False)
            return {}

        self.fake_direct.register_method("read", handle_read)
        self.fake_direct.register_method("write", handle_write)

        lab = DriverLab(
            self.fake_direct,
            grants_path=self.grants_path,
            evidence_root=self.evidence_root,
            target_scope="target-1",
            node_id="node-1",
        )

        async with await lab.attach("node-1", mode="direct") as session:
            self.assertEqual(session.mode, "direct")
            self.assertFalse(session.capabilities.target_policy)
            self.assertFalse(session.capabilities.target_audit)
            self.assertFalse(session.capabilities.target_local_timing)
            self.assertTrue(session.capabilities.production_driver_active)

            # Direct published protocol
            proto = await session.protocol("fuchsia.hardware.gpio/Device")
            res = await proto.read(resource="gpio")
            self.assertTrue(res.response["value"])

            # Gpio adapter
            gpio = await session.gpio("gpio")
            self.assertTrue(await gpio.read())
            await gpio.write(False)
            self.assertFalse(await gpio.read())

            # Attempting MMIO or target sequence in direct mode fails closed
            with self.assertRaises(UnsupportedCapabilityError):
                await session.mmio("control")

            with self.assertRaises(UnsupportedCapabilityError):
                await session.sequence([])

    async def test_proxy_mode_rejects_direct_protocols(self) -> None:
        lab = DriverLab(
            self.fake_proxy,
            grants_path=self.grants_path,
            evidence_root=self.evidence_root,
            target_scope="target-1",
            node_id="node-1",
        )
        async with await lab.attach("node-1", mode="proxy") as session:
            with self.assertRaises(UnsupportedCapabilityError):
                await session.protocol("fuchsia.hardware.gpio/Device")

            with self.assertRaises(UnsupportedCapabilityError):
                await session.gpio()

    async def test_selection_rules_in_attach(self) -> None:
        lab_proxy = DriverLab(
            self.fake_proxy,
            grants_path=self.grants_path,
            evidence_root=self.evidence_root,
            target_scope="target-1",
            node_id="node-1",
        )
        # auto with needs_mmio selects proxy
        session = await lab_proxy.attach(
            "node-1",
            mode="auto",
            requirements=AccessRequirements(needs_mmio=True),
        )
        self.assertEqual(session.mode, "proxy")
        await session.close()

        # direct mode with needs_mmio raises DriverLabError
        with self.assertRaises(DriverLabError):
            await lab_proxy.attach(
                "node-1",
                mode="direct",
                requirements=AccessRequirements(needs_mmio=True),
            )

        # Invalid mode string
        with self.assertRaises(ValueError):
            await lab_proxy.attach("node-1", mode="invalid")  # type: ignore[arg-type]

    async def test_mutating_session_consent_enforcement(self) -> None:
        # Consent denied
        prompt_denied = ScriptedPrompt(ConsentDecision.DENY_ONCE)
        lab_denied = DriverLab(
            self.fake_proxy,
            grants_path=self.grants_path,
            evidence_root=self.evidence_root,
            target_scope="target-1",
            node_id="node-1",
            consent=prompt_denied,
        )
        with self.assertRaises(DriverLabError):
            await lab_denied.attach(
                "node-1", mode="proxy", session_mode=SessionMode.MUTATING
            )

        # No consent prompt configured for mutating session
        lab_unattended = DriverLab(
            self.fake_proxy,
            grants_path=self.grants_path,
            evidence_root=self.evidence_root,
            target_scope="target-1",
            node_id="node-1",
            consent=None,
        )
        with self.assertRaises(DriverLabError):
            await lab_unattended.attach(
                "node-1", mode="proxy", session_mode=SessionMode.MUTATING
            )

    async def test_async_cancellation_cleans_up_session(self) -> None:
        prompt = ScriptedPrompt(ConsentDecision.ALLOW_ONCE)
        lab = DriverLab(
            self.fake_proxy,
            grants_path=self.grants_path,
            evidence_root=self.evidence_root,
            target_scope="target-1",
            node_id="node-1",
            consent=prompt,
        )

        session = await lab.attach(
            "node-1", mode="proxy", session_mode=SessionMode.MUTATING
        )
        self.assertEqual(self.fake_proxy.active_mutating_session, 1)

        async def worker() -> None:
            async with session:
                mmio = await session.mmio("control")
                await mmio.read32(0x10)
                await asyncio.sleep(10.0)

        task = asyncio.create_task(worker())
        await asyncio.sleep(0.01)
        task.cancel()

        with self.assertRaises(asyncio.CancelledError):
            await task

        self.assertTrue(session.is_closed)
        self.assertIsNone(self.fake_proxy.active_mutating_session)

        # Calling on closed session fails
        with self.assertRaises(UnsupportedCapabilityError):
            await session.mmio("control")

    async def test_representative_protocols_direct_mode(self) -> None:
        target = FakeDirectTarget(node_id="dev", protocol_name="multi")
        target.register_method("transfer", lambda a: {"read_data": [1, 2, 3]})
        target.register_method("transmit", lambda a: {"rx_data": [4, 5, 6]})
        target.register_method("write", lambda a: {"bytes_written": 2})
        target.register_method("read", lambda a: {"data": [7, 8]})
        target.register_method("enable", lambda a: {})
        target.register_method("disable", lambda a: {})
        target.register_method("assert_reset", lambda a: {})
        target.register_method("deassert_reset", lambda a: {})
        target.register_method("wait", lambda a: {"timestamp_ns": 12345})

        caps = SessionCapabilities(
            mode="direct",
            target_policy=False,
            target_audit=False,
            target_local_timing=False,
            fault_isolation="driver_host",
            production_driver_active=True,
        )
        desc = DirectDescription(
            node_id="dev", protocol_name="multi", boot_id="b1"
        )
        fake_session = await target.open_direct_session(None)  # type: ignore[arg-type]

        session = HardwareSession(
            capabilities=caps,
            direct_session=fake_session,
            direct_description=desc,
        )

        i2c = await session.i2c("i2c")
        rx_i2c = await i2c.transfer(b"abc", read_length=3)
        self.assertEqual(rx_i2c, bytes([1, 2, 3]))

        spi = await session.spi("spi")
        rx_spi = await spi.transmit(b"xyz")
        self.assertEqual(rx_spi, bytes([4, 5, 6]))

        serial = await session.serial("serial")
        written = await serial.write(b"hi")
        self.assertEqual(written, 2)
        rx_serial = await serial.read(2)
        self.assertEqual(rx_serial, bytes([7, 8]))

        clock = await session.clock("clock")
        await clock.enable()
        await clock.disable()

        reset = await session.reset("reset")
        await reset.assert_reset()
        await reset.deassert_reset()

        irq = await session.interrupt("irq")
        ts = await irq.wait(1.0)
        self.assertEqual(ts, 12345)

        await session.close()


if __name__ == "__main__":
    unittest.main()
