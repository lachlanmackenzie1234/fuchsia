// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "src/devices/usb/drivers/dwc3/dwc3-test-fixture.h"

namespace dwc3 {

TEST_F(UnmanagedTestFixture, Ep0Lifecycle) {
  SetUpAndPowerOnDriver();

  // Start the controller to bring up the device fully (this triggers StartPeripheralMode).
  auto controller_client_end = dut_.Connect<fuchsia_hardware_usb_dci::UsbDciService::Device>();
  ASSERT_TRUE(controller_client_end.is_ok());
  fidl::SyncClient controller_client(std::move(controller_client_end.value()));
  auto start_result = controller_client->StartController();
  ASSERT_TRUE(start_result.is_ok());

  // Verify that hardware initialization has completed by checking the DCTL RUN_STOP bit.
  bool is_run_stop_set = false;
  dut_.RunInEnvironmentTypeContext([&](Environment& env) {
    uint32_t val = static_cast<uint32_t>(env.reg_region()[DCTL::Get().addr()].Read());
    is_run_stop_set = DCTL::Get().FromValue(val).RUN_STOP();
  });
  EXPECT_TRUE(is_run_stop_set);

  // Verify Ep0Start() completed and initial Setup TRB was queued.
  dut_.RunInDriverContext([&](Dwc3& drv) {
    EXPECT_EQ(Dwc3TestHelper::GetEp0State(drv), Dwc3TestHelper::State::Setup);
  });

  dut_.RunInNodeContext(
      [&](fdf_testing::TestNode& node) { EXPECT_EQ(1UL, node.children().size()); });

  TearDownAndPowerOffDriver();
}

TEST_F(UnmanagedTestFixture, NoPrematureWritesOnGetDescriptor) {
  SetUpAndPowerOnDriver();

  bool write_detected = false;
  FakeUsbDciInterface fake_dci;
  auto binding = BindDciInterface(&fake_dci);

  dut_.RunInDriverContext([&](Dwc3& drv) {
    Dwc3TestHelper::Ep0QueueSetup(drv);
    Dwc3TestHelper::SetEp0State(drv, Dwc3TestHelper::State::Setup);
    Dwc3TestHelper::SetControllerStarted(drv, true);

    auto setup = MakeGetDescriptorSetup();

    void* buf = Dwc3TestHelper::GetEp0BufferVirt(drv);
    std::memcpy(buf, &setup, sizeof(setup));
  });

  libsync::Completion data_phase_started;

  dut_.RunInEnvironmentTypeContext([&](Environment& env) {
    env.reg_region()[DCTL::Get().addr()].SetWriteCallback([&](uint64_t val_raw) {
      [[maybe_unused]] uint32_t val = static_cast<uint32_t>(val_raw);
      fdf::info("DCTL write: 0x{:x}", val);
      write_detected = true;
    });
    env.reg_region()[DCFG::Get().addr()].SetWriteCallback([&](uint64_t val_raw) {
      [[maybe_unused]] uint32_t val = static_cast<uint32_t>(val_raw);
      fdf::info("DCFG write: 0x{:x}", val);
      write_detected = true;
    });
    env.reg_region()[DEPCMD::Get(1).addr()].SetWriteCallback(
        [&, called = false](uint64_t val_raw) mutable {
          [[maybe_unused]] uint32_t val = static_cast<uint32_t>(val_raw);
          if (!called) {
            called = true;
            data_phase_started.Signal();
          }
        });
  });

  dut_.RunInDriverContext([&](Dwc3& drv) {
    Dwc3TestHelper::HandleEp0TransferCompleteEvent(drv, 0);  // 0 is kEp0Out
  });

  ASSERT_EQ(ZX_OK, data_phase_started.Wait(zx::sec(5)));

  dut_.RunInDriverContext([&](Dwc3& drv) { EXPECT_FALSE(write_detected); });

  dut_.RunInEnvironmentTypeContext([&](Environment& env) {
    env.reg_region()[DCTL::Get().addr()].SetWriteCallback(nullptr);
    env.reg_region()[DCFG::Get().addr()].SetWriteCallback(nullptr);
    env.reg_region()[DEPCMD::Get(1).addr()].SetWriteCallback(nullptr);
  });

  if (binding.has_value()) {
    binding->Unbind();
    dut_.runtime().RunUntilIdle();
  }

  TearDownAndPowerOffDriver();
}

TEST_F(UnmanagedTestFixture, InvalidSetupRequest) {
  SetUpAndPowerOnDriver();

  FakeUsbDciInterface fake_dci;
  fake_dci.SetControlStatus(ZX_ERR_NOT_SUPPORTED);
  auto binding = BindDciInterface(&fake_dci);

  dut_.RunInDriverContext([&](Dwc3& drv) {
    Dwc3TestHelper::Ep0QueueSetup(drv);
    Dwc3TestHelper::SetEp0State(drv, Dwc3TestHelper::State::Setup);
    Dwc3TestHelper::SetControllerStarted(drv, true);
    fake_dci.SetControlStatus(ZX_ERR_NOT_SUPPORTED);

    auto setup = MakeSetupPacket(0xC0, 0x99, 0, 0, 8);

    void* buf = Dwc3TestHelper::GetEp0BufferVirt(drv);
    std::memcpy(buf, &setup, sizeof(setup));

    // Simulate Setup phase completion
    Dwc3TestHelper::HandleEp0TransferCompleteEvent(drv, 0);
  });

  EXPECT_TRUE(dut_.runtime().RunWithTimeoutOrUntil(
      [&]() {
        bool done = false;
        dut_.RunInDriverContext([&](Dwc3& drv) {
          done = (Dwc3TestHelper::GetEp0State(drv) == Dwc3TestHelper::State::Setup);
        });
        return done;
      },
      zx::sec(10)));

  dut_.RunInDriverContext([&](Dwc3& drv) {
    // Verification: It should have failed and returned to Setup state
    EXPECT_EQ(Dwc3TestHelper::GetEp0State(drv), Dwc3TestHelper::State::Setup);
  });

  if (binding.has_value()) {
    binding->Unbind();
    dut_.runtime().RunUntilIdle();
  }

  TearDownAndPowerOffDriver();
}

TEST_F(UnmanagedTestFixture, OppositeDirectionXferNotReady) {
  SetUpAndPowerOnDriver();

  dut_.RunInDriverContext([&](Dwc3& drv) {
    Dwc3TestHelper::SetControllerStarted(drv, true);

    // Simulate DataOut state (OUT transfer in progress)
    Dwc3TestHelper::SetEp0State(drv, Dwc3TestHelper::State::DataOut);
    Dwc3TestHelper::SetEpTransferState(
        drv, 0, Dwc3TestHelper::TransferState::kActiveSingle);  // EP0 OUT in progress
    Dwc3TestHelper::SetEpRsrcId(drv, 0, 1);                     // Set valid resource ID

    // Simulate XferNotReady for EP0 IN (opposite direction)
    Dwc3TestHelper::HandleEp0TransferNotReadyEvent(drv, 1, DEPEVT_XFER_NOT_READY_STAGE_DATA);

    // Verification: It should have called fail() logic:
    // - Stalled EP0 OUT
    // - Queued Setup
    // And it should have returned to Setup state.
    EXPECT_EQ(Dwc3TestHelper::GetEp0State(drv), Dwc3TestHelper::State::Setup);
  });

  TearDownAndPowerOffDriver();
}

// TODO(b/509735595): Re-enable once the production fixes are landed.
TEST_F(UnmanagedTestFixture, DISABLED_ResetDuringControlWrite) {
  SetUpAndPowerOnDriver();

  dut_.RunInDriverContext([&](Dwc3& drv) {
    Dwc3TestHelper::SetControllerStarted(drv, true);

    // Simulate DataOut state (OUT transfer in progress)
    Dwc3TestHelper::SetEp0State(drv, Dwc3TestHelper::State::DataOut);
    Dwc3TestHelper::SetEpTransferState(
        drv, 0, Dwc3TestHelper::TransferState::kActiveSingle);  // EP0 OUT in progress
    // Do NOT set rsrc_id to simulate uninitialized resource ID during transfer!
  });

  // Stop driver while transfer is in progress.
  // This should trigger Ep0Reset, which will call CmdEpEndTransfer.
  // If rsrc_id is invalid, it will assert/crash!
  TearDownAndPowerOffDriver();
}

// TODO(b/509735595): Re-enable once the production fixes are landed.
TEST_F(UnmanagedTestFixture, DISABLED_Ep0ResetForceAbortsOutstandingSetupTrb) {
  // Shared sequential log to verify strict protocol stages chronologically!
  auto sequence_log = std::make_shared<std::vector<std::string>>();

  SetUpAndPowerOnDriver();

  dut_.RunInEnvironmentTypeContext([&, sequence_log](Environment& env) {
    // Mock DEPCMD for EP0 OUT to capture the force End Transfer write!
    env.reg_region()[DEPCMD::Get(0).addr()].SetWriteCallback([sequence_log](uint64_t val_raw) {
      [[maybe_unused]] uint32_t val = static_cast<uint32_t>(val_raw);
      if (DEPCMD::Get(0).FromValue(val).CMDTYP() == DEPCMD::DEPENDXFER) {
        sequence_log->push_back("END_XFER_EP0_OUT");
      }
    });

    // Mock DEPCMD for EP0 IN to capture the force End Transfer write!
    env.reg_region()[DEPCMD::Get(1).addr()].SetWriteCallback([sequence_log](uint64_t val_raw) {
      [[maybe_unused]] uint32_t val = static_cast<uint32_t>(val_raw);
      if (DEPCMD::Get(1).FromValue(val).CMDTYP() == DEPCMD::DEPENDXFER) {
        sequence_log->push_back("END_XFER_EP0_IN");
      }
    });
  });

  dut_.RunInDriverContext([&](Dwc3& drv) {
    Dwc3TestHelper::SetControllerStarted(drv, true);

    // Place EP0 in Setup state waiting for packets
    Dwc3TestHelper::SetEp0State(drv, Dwc3TestHelper::State::Setup);
    Dwc3TestHelper::SetEpTransferState(
        drv, 0, Dwc3TestHelper::TransferState::kIdle);  // Outstanding Setup TRB queued!

    // Invoke Ep0Reset! This must unconditionally and sequentially abort EP0 OUT then EP0 IN!
    Dwc3TestHelper::Ep0Reset(drv);
  });

  // Verify the strict, chronological protocol sequence!
  ASSERT_EQ(sequence_log->size(), 2u);
  EXPECT_EQ((*sequence_log)[0], "END_XFER_EP0_OUT")
      << "Protocol Violation: Setup phase force-abort must be written to EP0 OUT first!";
  EXPECT_EQ((*sequence_log)[1], "END_XFER_EP0_IN")
      << "Protocol Violation: Setup phase force-abort must be written to EP0 IN second!";

  TearDownAndPowerOffDriver();
}

TEST_F(UnmanagedTestFixture, EndTransferOnValidResource) {
  bool depcmd_written = false;
  uint32_t depcmd_val = 0;

  SetUpAndPowerOnDriver();

  dut_.RunInEnvironmentTypeContext([&](Environment& env) {
    // Mock DEPCMD for EP0 OUT to detect the "End Transfer" command!
    env.reg_region()[DEPCMD::Get(0).addr()].SetWriteCallback([&](uint64_t val_raw) {
      [[maybe_unused]] uint32_t val = static_cast<uint32_t>(val_raw);
      if (DEPCMD::Get(0).FromValue(val).CMDTYP() == DEPCMD::DEPENDXFER) {
        depcmd_written = true;
        depcmd_val = val;
      }
    });
  });

  dut_.RunInDriverContext([&](Dwc3& drv) {
    Dwc3TestHelper::SetControllerStarted(drv, true);

    // Simulate DataOut state and SET A VALID RESOURCE ID!
    Dwc3TestHelper::SetEp0State(drv, Dwc3TestHelper::State::DataOut);
    Dwc3TestHelper::SetEpTransferState(drv, 0, Dwc3TestHelper::TransferState::kActiveSingle);
    Dwc3TestHelper::SetEpRsrcId(drv, 0, 2);  // Valid resource ID
  });

  // Manually trigger the "End Transfer" logic by calling Ep0Reset directly!
  dut_.RunInDriverContext([&](Dwc3& drv) { Dwc3TestHelper::Ep0Reset(drv); });

  // Now that we've captured the write, we can let the fixture finish the official stop sequence.
  TearDownAndPowerOffDriver();

  // Verification: Driver should have written the End Transfer command to DEPCMD!
  EXPECT_TRUE(depcmd_written);
}

// TODO(b/509735595): Re-enable once the production fixes are landed.
TEST_F(UnmanagedTestFixture, DISABLED_EndTransferOnBothEndpoints) {
  bool ep0_out_end_transfer = false;
  bool ep0_in_end_transfer = false;

  SetUpAndPowerOnDriver();

  dut_.RunInEnvironmentTypeContext([&](Environment& env) {
    // Mock DEPCMD for EP0 OUT (0) to detect End Transfer and assert COMMANDPARAM
    env.reg_region()[DEPCMD::Get(0).addr()].SetWriteCallback([&](uint64_t val_raw) {
      [[maybe_unused]] uint32_t val = static_cast<uint32_t>(val_raw);
      if (DEPCMD::Get(0).FromValue(val).CMDTYP() == DEPCMD::DEPENDXFER) {
        ep0_out_end_transfer = true;
        const uint32_t param = DEPCMD::Get(0).FromValue(val).COMMANDPARAM();
        if (param != 0) {
          EXPECT_EQ(param, 2u);
        }
      }
    });

    // Mock DEPCMD for EP0 IN (1) to detect End Transfer and assert COMMANDPARAM
    env.reg_region()[DEPCMD::Get(1).addr()].SetWriteCallback([&](uint64_t val_raw) {
      [[maybe_unused]] uint32_t val = static_cast<uint32_t>(val_raw);
      if (DEPCMD::Get(1).FromValue(val).CMDTYP() == DEPCMD::DEPENDXFER) {
        ep0_in_end_transfer = true;
        const uint32_t param = DEPCMD::Get(1).FromValue(val).COMMANDPARAM();
        if (param != 0) {
          EXPECT_EQ(param, 2u);
        }
      }
    });
  });

  dut_.runtime().RunUntilIdle();

  // Setup the active transfer states for both directions
  dut_.RunInDriverContext([&](Dwc3& drv) {
    Dwc3TestHelper::SetControllerStarted(drv, true);

    // Simulate an active state
    Dwc3TestHelper::SetEp0State(drv, Dwc3TestHelper::State::DataOut);

    // Set transfer_state to active and assign valid resource IDs for both EP0 IN and EP0 OUT
    Dwc3TestHelper::SetEpTransferState(drv, 0, Dwc3TestHelper::TransferState::kActiveSingle);
    Dwc3TestHelper::SetEpRsrcId(drv, 0, 2);

    Dwc3TestHelper::SetEpTransferState(drv, 1, Dwc3TestHelper::TransferState::kActiveSingle);
    Dwc3TestHelper::SetEpRsrcId(drv, 1, 2);
  });

  // Manually trigger the Reset logic which should end transfers
  dut_.RunInDriverContext([&](Dwc3& drv) { Dwc3TestHelper::Ep0Reset(drv); });

  // Complete fixture teardown safely
  TearDownAndPowerOffDriver();

  // Verification
  EXPECT_TRUE(ep0_out_end_transfer);
  EXPECT_TRUE(ep0_in_end_transfer);
}

TEST_F(UnmanagedTestFixture, FidlControlCallFailure) {
  SetUpAndPowerOnDriver();

  BindDciInterfaceWithoutServer();

  dut_.RunInDriverContext([&](Dwc3& drv) {
    Dwc3TestHelper::Ep0QueueSetup(drv);
    Dwc3TestHelper::SetEp0State(drv, Dwc3TestHelper::State::Setup);
    Dwc3TestHelper::SetControllerStarted(drv, true);
  });

  dut_.RunInDriverContext([&](Dwc3& drv) {
    auto setup = MakeSetupPacket(0xC0, 0x99, 0, 0, 8);

    void* buf = Dwc3TestHelper::GetEp0BufferVirt(drv);
    std::memcpy(buf, &setup, sizeof(setup));

    // Simulate Setup phase completion
    Dwc3TestHelper::HandleEp0TransferCompleteEvent(drv, 0);
  });

  EXPECT_TRUE(dut_.runtime().RunWithTimeoutOrUntil(
      [&]() {
        bool done = false;
        dut_.RunInDriverContext([&](Dwc3& drv) {
          done = (Dwc3TestHelper::GetEp0State(drv) == Dwc3TestHelper::State::Setup);
        });
        return done;
      },
      zx::sec(10)));

  dut_.RunInDriverContext([&](Dwc3& drv) {
    // Verification: It should have failed and returned to Setup state
    EXPECT_EQ(Dwc3TestHelper::GetEp0State(drv), Dwc3TestHelper::State::Setup);
  });

  TearDownAndPowerOffDriver();
}

// TODO(b/509735595): Re-enable once the production fixes are landed.
TEST_F(UnmanagedTestFixture, DISABLED_UsbBusResetDuringTransfer) {
  SetUpAndPowerOnDriver();

  dut_.RunInDriverContext([&](Dwc3& drv) {
    Dwc3TestHelper::SetControllerStarted(drv, true);

    // Simulate DataOut state (OUT transfer in progress)
    Dwc3TestHelper::SetEp0State(drv, Dwc3TestHelper::State::DataOut);
    Dwc3TestHelper::SetEpTransferState(
        drv, 0, Dwc3TestHelper::TransferState::kActiveSingle);  // EP0 OUT in progress
    Dwc3TestHelper::SetEpRsrcId(drv, 0, 1);                     // Set valid resource ID

    // Trigger Bus Reset
    Dwc3TestHelper::HandleResetEvent(drv);

    // Verification: State should be reset to Setup
    EXPECT_EQ(Dwc3TestHelper::GetEp0State(drv), Dwc3TestHelper::State::Setup);
  });

  TearDownAndPowerOffDriver();
}

// TODO(b/509735595): Re-enable once the production fixes are landed.
TEST_F(UnmanagedTestFixture, DISABLED_DisableEndpointDuringTransfer) {
  SetUpAndPowerOnDriver();

  dut_.RunInEnvironmentTypeContext([](Environment& env) {
    // Mock DEPCMD for EP2 to return 0 (Command Complete)
    env.reg_region()[DEPCMD::Get(2).addr()].SetReadCallback([]() -> uint32_t { return 0; });
  });

  dut_.RunInDriverContext([&](Dwc3& drv) {
    Dwc3TestHelper::SetControllerStarted(drv, true);

    // Access user endpoint 2
    auto* uep = Dwc3TestHelper::GetUserEndpoint(drv, 2);
    ASSERT_NE(uep, nullptr);

    // Simulate transfer in progress
    Dwc3TestHelper::SetEpTransferState(drv, 2, Dwc3TestHelper::TransferState::kActiveSingle);
    // Do NOT set rsrc_id (leave invalid)

    // Simulate function driver calling DisableEndpoint
    // We call EpSetConfig(..., false) directly as it is the core of DisableEndpoint
    // and doesn't require complex FIDL mocking.
    Dwc3TestHelper::EpSetConfig(drv, uep->ep, false);
  });

  // StopDriver calls Stop() which calls ResetEndpoints()
  // ResetEndpoints should call CmdEpEndTransfer on EP2 because transfer_state is active!
  // And since rsrc_id is invalid, it should crash!
  TearDownAndPowerOffDriver();
}

TEST_F(UnmanagedTestFixture, ControllerStoppedDuringTransfer) {
  SetUpAndPowerOnDriver();

  bool gevntcount_read = false;
  auto read_count = std::make_shared<std::atomic<int>>(0);
  dut_.RunInEnvironmentTypeContext([&, read_count](Environment& env) {
    env.reg_region()[GEVNTCOUNT::Get(0).addr()].SetReadCallback([&, read_count]() -> uint32_t {
      gevntcount_read = true;
      if (read_count->fetch_add(1) == 0) {
        return 4;
      }
      return 0;
    });
  });

  dut_.RunInDriverContext([&](Dwc3& drv) {
    // Set controller_started_ = false
    Dwc3TestHelper::SetControllerStarted(drv, false);

    // Call HandleIrq directly via helper
    // Pass placeholder arguments
    Dwc3TestHelper::HandleIrq(drv, nullptr, nullptr, ZX_OK, nullptr);
  });

  // Verification: GEVNTCOUNT should NOT have been read!
  EXPECT_FALSE(gevntcount_read);

  TearDownAndPowerOffDriver();
}

// This test verifies that a control transfer with wLength = 0 (Zero Length Packet)
// skips the Data stage and proceeds directly to the Status stage.
TEST_F(UnmanagedTestFixture, ZeroLengthPacket) {
  FakeUsbDciInterface fake_dci;

  SetUpAndPowerOnDriver();

  std::atomic<bool> callback_executed{false};
  fake_dci.SetControlCallback(
      [&](fuchsia_hardware_usb_descriptor::wire::UsbSetup setup, cpp20::span<const uint8_t> data) {
        callback_executed.store(true);
      });

  auto binding = BindDciInterface(&fake_dci);

  dut_.RunInDriverContext([&](Dwc3& drv) {
    Dwc3TestHelper::SetControllerStarted(drv, true);
    Dwc3TestHelper::SetEp0State(drv, Dwc3TestHelper::State::Setup);

    // Setup packet with wLength = 0
    auto setup = MakeSetupPacket(USB_DIR_OUT | USB_TYPE_VENDOR | USB_RECIP_DEVICE, 0x01, 0, 0, 0);

    std::memcpy(Dwc3TestHelper::GetEp0BufferVirt(drv), &setup, sizeof(setup));

    // Push placeholder TRB to avoid AdvanceRead error
    dwc3_trb_t trb{};
    Dwc3TestHelper::PushTrbToSharedFifo(drv, trb);

    // Call HandleEp0TransferCompleteEvent
    Dwc3TestHelper::HandleEp0TransferCompleteEvent(drv, 0);
  });

  // Wait for state transition to WaitHost deterministically
  EXPECT_TRUE(dut_.runtime().RunWithTimeoutOrUntil(
      [&]() {
        bool done = false;
        dut_.RunInDriverContext([&](Dwc3& drv) {
          auto state = Dwc3TestHelper::GetEp0State(drv);
          done = (state == Dwc3TestHelper::State::WaitHost);
          if (state == Dwc3TestHelper::State::Setup) {
            done = true;
          }
        });
        return done;
      },
      zx::sec(10)));

  dut_.RunInDriverContext([&](Dwc3& drv) {
    // Verification: State should become WaitHost (after TwoStage)
    EXPECT_EQ(Dwc3TestHelper::GetEp0State(drv), Dwc3TestHelper::State::WaitHost);
  });

  if (binding.has_value()) {
    binding->Unbind();
    dut_.runtime().RunUntilIdle();
  }

  TearDownAndPowerOffDriver();
}

// This test verifies that the driver handles a Control Write with maximum buffer size.
// TODO(b/509735595): Re-enable once the production fixes are landed.
TEST_F(UnmanagedTestFixture, DISABLED_MaxBufferSizeTransfer) {
  FakeUsbDciInterface fake_dci;

  SetUpAndPowerOnDriver();

  std::atomic<size_t> received_length{0};
  fake_dci.SetControlCallback(
      [&](fuchsia_hardware_usb_descriptor::wire::UsbSetup setup, cpp20::span<const uint8_t> data) {
        received_length.store(data.size());
      });

  auto binding = BindDciInterface(&fake_dci);

  dut_.RunInDriverContext([&](Dwc3& drv) {
    Dwc3TestHelper::SetControllerStarted(drv, true);
    Dwc3TestHelper::SetEp0OutEnabled(drv, true);
    Dwc3TestHelper::Ep0QueueSetup(drv);
    Dwc3TestHelper::SetEp0State(drv, Dwc3TestHelper::State::Setup);

    // Use high-level construct to simulate Setup received!
    Dwc3TestHelper::SimulateSetupReceived(
        drv, MakeSetupPacket(USB_DIR_OUT | USB_TYPE_VENDOR | USB_RECIP_DEVICE, 0x01, 0, 0, 65535));
  });

  dut_.RunInDriverContext([&](Dwc3& drv) {
    // With the FIDL limit fix, oversized requests are stalled and state returns to Setup.
    EXPECT_EQ(Dwc3TestHelper::GetEp0State(drv), Dwc3TestHelper::State::Setup);
  });

  if (binding.has_value()) {
    binding->Unbind();
    dut_.runtime().RunUntilIdle();
  }

  TearDownAndPowerOffDriver();
}

// This test verifies that the driver handles a Control Read that requires a ZLP.
// Note: This test reveals that the driver state machine is broken for Control Read.
// It transitions from WaitFidl directly to Status, even though it just queued
// a Data phase transfer. It also does not send a ZLP when required.
// TODO(b/509735595): Re-enable once the production fixes are landed.
TEST_F(UnmanagedTestFixture, DISABLED_ZlpInTransferRequired) {
  FakeUsbDciInterface fake_dci;

  SetUpAndPowerOnDriver();

  // Simulate device returning 512 bytes (MPS = 512)
  std::vector<uint8_t> read_data(512, 0xAA);
  fake_dci.SetReadData(read_data);

  auto binding = BindDciInterface(&fake_dci);

  dut_.RunInDriverContext([&](Dwc3& drv) {
    Dwc3TestHelper::SetControllerStarted(drv, true);
    Dwc3TestHelper::Ep0QueueSetup(drv);
    Dwc3TestHelper::SetEp0State(drv, Dwc3TestHelper::State::Setup);

    // Setup packet with wLength = 1024
    auto setup = MakeSetupPacket(USB_DIR_IN | USB_TYPE_VENDOR | USB_RECIP_DEVICE, 0x01, 0, 0, 1024);

    // Write setup packet to buffer
    void* buf = Dwc3TestHelper::GetEp0BufferVirt(drv);
    std::memcpy(buf, &setup, sizeof(setup));

    // Call HandleEp0TransferCompleteEvent to trigger HandleSetup
    Dwc3TestHelper::HandleEp0TransferCompleteEvent(drv, 0);  // 0 is kEp0Out
  });

  // Wait for callback to be executed deterministically
  EXPECT_TRUE(dut_.runtime().RunWithTimeoutOrUntil(
      [&]() {
        bool ready = false;
        dut_.RunInDriverContext([&](Dwc3& drv) { ready = !Dwc3TestHelper::IsFifoEmpty(drv); });
        return ready;
      },
      zx::sec(10)));

  dut_.RunInDriverContext([&](Dwc3& drv) {
    // Verification: State should be DataIn (correct behavior)
    EXPECT_EQ(Dwc3TestHelper::GetEp0State(drv), Dwc3TestHelper::State::DataIn);

    Dwc3TestHelper::SimulateDataInPhase(drv);

    // Simulate ZLP completion on EP0 IN!
    Dwc3TestHelper::SimulateDataInPhase(drv);

    // Verification: State should be WaitNrdyOut
    EXPECT_EQ(Dwc3TestHelper::GetEp0State(drv), Dwc3TestHelper::State::WaitNrdyOut);
  });

  if (binding.has_value()) {
    binding->Unbind();
    dut_.runtime().RunUntilIdle();
  }

  TearDownAndPowerOffDriver();
}

TEST_F(UnmanagedTestFixture, UsbResetTransitionsToDefaultState) {
  SetUpAndPowerOnDriver();

  auto client_end = dut_.Connect<fuchsia_hardware_usb_policy::Service::Controller>();
  ASSERT_TRUE(client_end.is_ok());
  fidl::SyncClient client{std::move(*client_end)};

  // 1. Consume initial state.
  auto watch_result1 = client->WatchDeviceState();
  ASSERT_TRUE(watch_result1.is_ok());

  // 2. Set device address to 5!
  dut_.RunInDriverContext([](Dwc3& drv) { Dwc3TestHelper::SetDeviceAddress(drv, 5); });

  // 3. Verify state is kAddress via WatchDeviceState!
  auto watch_result2 = client->WatchDeviceState();
  ASSERT_TRUE(watch_result2.is_ok());
  ASSERT_TRUE(watch_result2->state().has_value());
  EXPECT_EQ(watch_result2->state().value(), fuchsia_hardware_usb_policy::DeviceState::kAddress);
  ASSERT_TRUE(watch_result2->address().has_value());
  EXPECT_EQ(watch_result2->address().value(), 5u);

  // 4. Simulate USB Reset event!
  dut_.RunInDriverContext([](Dwc3& drv) { Dwc3TestHelper::HandleEvent(drv, 257); });

  // 5. Verify state is kDefault via WatchDeviceState!
  auto watch_result3 = client->WatchDeviceState();
  ASSERT_TRUE(watch_result3.is_ok());
  ASSERT_TRUE(watch_result3->state().has_value());
  EXPECT_EQ(watch_result3->state().value(), fuchsia_hardware_usb_policy::DeviceState::kDefault);
  ASSERT_TRUE(watch_result3->address().has_value());
  EXPECT_EQ(watch_result3->address().value(), 0u);

  TearDownAndPowerOffDriver();
}

TEST_F(UnmanagedTestFixture, SetInterfaceAlreadySet) {
  SetUpAndPowerOnDriver();

  auto client_end = dut_.Connect<fuchsia_hardware_usb_dci::UsbDciService::Device>();
  ASSERT_TRUE(client_end.is_ok());
  fidl::SyncClient client{std::move(*client_end)};

  // Create a valid interface client!
  auto [interface_client, interface_server] =
      fidl::Endpoints<fuchsia_hardware_usb_dci::UsbDciInterface>::Create();

  // Call SetInterface first time!
  auto result = client->SetInterface({{.interface = std::move(interface_client)}});
  ASSERT_TRUE(result.is_ok());

  // Create another valid interface client!
  auto [interface_client2, interface_server2] =
      fidl::Endpoints<fuchsia_hardware_usb_dci::UsbDciInterface>::Create();

  // Call SetInterface second time!
  auto result2 = client->SetInterface({{.interface = std::move(interface_client2)}});

  ASSERT_TRUE(result2.is_error());
  ASSERT_TRUE(result2.error_value().is_domain_error());
  EXPECT_EQ(result2.error_value().domain_error(), ZX_ERR_BAD_STATE);

  TearDownAndPowerOffDriver();
}

// TODO(b/509735595): Re-enable once the production fixes are landed.
TEST_F(UnmanagedTestFixture, DISABLED_SpuriousTransferCompleteWithHwoSetTriggersStallAbort) {
  SetUpAndPowerOnDriver();
  dut_.RunInDriverContext([&](Dwc3& drv) {
    Dwc3TestHelper::SetControllerStarted(drv, true);
    Dwc3TestHelper::SetEp0State(drv, Dwc3TestHelper::State::Setup);

    // Ensure FIFO is cleared!
    Dwc3TestHelper::ClearSharedFifo(drv);

    // Call via helper. Safe early return should occur without AdvanceRead failure log.
    Dwc3TestHelper::SimulateGhostTransferCompleteEvent(drv, 0);

    // State should remain stable.
    EXPECT_EQ(Dwc3TestHelper::GetEp0State(drv), Dwc3TestHelper::State::Setup);
  });

  TearDownAndPowerOffDriver();
}

// Verifies hardware race condition recovery during Transfer Complete processing.
// In practice, if a Transfer Complete interrupt fires while the TRB's Hardware Own (HWO) bit
// is still set (e.g. due to DMA cache latency or a spurious hardware event), the driver
// retries reading the TRB. If HWO remains set after retries, the driver safely aborts the
// transfer, stalls the endpoint, and resets EP0 to the Setup state.
TEST_F(UnmanagedTestFixture, SpuriousTransferCompleteWithHwoSetTriggersStallAbort) {
  SetUpAndPowerOnDriver();
  dut_.RunInDriverContext([&](Dwc3& drv) {
    Dwc3TestHelper::SetControllerStarted(drv, true);
    Dwc3TestHelper::SetEp0OutEnabled(drv, true);
    Dwc3TestHelper::SetEp0State(drv, Dwc3TestHelper::State::DataOut);

    // Push a "Dirty" TRB that specifies HWO is still set!
    dwc3_trb_t trb{};
    trb.control = TRB_HWO;
    Dwc3TestHelper::PushTrbToSharedFifo(drv, trb);

    // Call via helper wrapper (bypasses standard wrapper's auto-clear logic).
    // This forces 5 retries and then absolute abort.
    Dwc3TestHelper::SimulateGhostTransferCompleteEvent(drv, 0);

    // Assert abort logic fired!
    EXPECT_TRUE(Dwc3TestHelper::IsEp0OutStalled(drv));
    EXPECT_EQ(Dwc3TestHelper::GetEp0State(drv), Dwc3TestHelper::State::Setup);
  });

  TearDownAndPowerOffDriver();
}

TEST_F(UnmanagedTestFixture, ControlReadParseGetDescriptor) {
  SetUpAndPowerOnDriver();

  FakeUsbDciInterface fake_dci;
  libsync::Completion control_called;
  fake_dci.SetControlCallback([&](fuchsia_hardware_usb_descriptor::wire::UsbSetup setup,
                                  cpp20::span<const uint8_t> data) { control_called.Signal(); });

  std::vector<uint8_t> read_data(18, 0xAA);
  fake_dci.SetReadData(std::move(read_data));

  libsync::Completion data_phase_started;
  dut_.RunInEnvironmentTypeContext([&](Environment& env) {
    env.reg_region()[DEPCMD::Get(1).addr()].SetWriteCallback(
        [&, called = false](uint64_t val_raw) mutable {
          [[maybe_unused]] uint32_t val = static_cast<uint32_t>(val_raw);
          if (!called) {
            called = true;
            fdf::info("DEPCMD write detected!");
            data_phase_started.Signal();
          }
        });
  });

  auto binding = BindDciInterface(&fake_dci);

  dut_.RunInDriverContext([&](Dwc3& drv) {
    Dwc3TestHelper::SetEp0OutEnabled(drv, true);
    Dwc3TestHelper::SetEp0InEnabled(drv, true);
    Dwc3TestHelper::Ep0QueueSetup(drv);
    Dwc3TestHelper::SetEp0State(drv, Dwc3TestHelper::State::Setup);
    Dwc3TestHelper::SetControllerStarted(drv, true);

    auto setup = MakeGetDescriptorSetup(18);
    void* buf = Dwc3TestHelper::GetEp0BufferVirt(drv);
    std::memcpy(buf, &setup, sizeof(setup));
    Dwc3TestHelper::HandleEp0TransferCompleteEvent(drv, 0);  // 0 is kEp0Out
  });

  ASSERT_EQ(ZX_OK, data_phase_started.Wait(zx::sec(5)));
  dut_.RunInDriverContext([&](Dwc3& drv) {
    auto state = Dwc3TestHelper::GetEp0State(drv);
    EXPECT_TRUE(state == Dwc3TestHelper::State::DataIn || state == Dwc3TestHelper::State::Setup);
  });
  EXPECT_TRUE(fake_dci.control_called());

  if (binding.has_value()) {
    binding->Unbind();
    dut_.runtime().RunUntilIdle();
  }

  TearDownAndPowerOffDriver();
}

TEST_F(UnmanagedTestFixture, ControlReadCompleteGetDescriptor) {
  SetUpAndPowerOnDriver();

  FakeUsbDciInterface fake_dci;
  libsync::Completion control_called;
  fake_dci.SetControlCallback([&](fuchsia_hardware_usb_descriptor::wire::UsbSetup setup,
                                  cpp20::span<const uint8_t> data) { control_called.Signal(); });

  std::vector<uint8_t> read_data(18, 0xAA);
  fake_dci.SetReadData(std::move(read_data));

  auto binding = BindDciInterface(&fake_dci);

  dut_.RunInDriverContext([&](Dwc3& drv) {
    Dwc3TestHelper::SetEp0OutEnabled(drv, true);
    Dwc3TestHelper::SetEp0InEnabled(drv, true);
    Dwc3TestHelper::Ep0Reset(drv);
    Dwc3TestHelper::Ep0QueueSetup(drv);
    Dwc3TestHelper::SetEp0State(drv, Dwc3TestHelper::State::Setup);
    Dwc3TestHelper::SetControllerStarted(drv, true);

    auto setup = MakeGetDescriptorSetup(18);
    void* buf = Dwc3TestHelper::GetEp0BufferVirt(drv);
    std::memcpy(buf, &setup, sizeof(setup));
    Dwc3TestHelper::HandleEp0TransferCompleteEvent(drv, 0);  // 0 is kEp0Out
  });

  ASSERT_EQ(ZX_OK, control_called.Wait(zx::sec(5)));

  if (binding.has_value()) {
    binding->Unbind();
    dut_.runtime().RunUntilIdle();
  }

  TearDownAndPowerOffDriver();
}

TEST_F(UnmanagedTestFixture, ControlReadShortPacketDataIn) {
  SetUpAndPowerOnDriver();

  FakeUsbDciInterface fake_dci;
  libsync::Completion control_called;
  fake_dci.SetControlCallback([&](fuchsia_hardware_usb_descriptor::wire::UsbSetup setup,
                                  cpp20::span<const uint8_t> data) { control_called.Signal(); });

  std::vector<uint8_t> read_data(32, 0xAA);
  fake_dci.SetReadData(std::move(read_data));

  auto binding = BindDciInterface(&fake_dci);

  dut_.RunInDriverContext([&](Dwc3& drv) {
    Dwc3TestHelper::SetEp0OutEnabled(drv, true);
    Dwc3TestHelper::SetEp0InEnabled(drv, true);
    Dwc3TestHelper::Ep0QueueSetup(drv);
    Dwc3TestHelper::SetEp0State(drv, Dwc3TestHelper::State::Setup);
    Dwc3TestHelper::SetControllerStarted(drv, true);

    auto setup = MakeGetDescriptorSetup(64);
    setup.bm_request_type = USB_DIR_IN | USB_TYPE_VENDOR | USB_RECIP_DEVICE;
    setup.b_request = 0x01;
    Dwc3TestHelper::SimulateSetupReceived(drv, setup);
  });

  // Wait for the driver to actually queue the data phase transfer to the hardware!
  EXPECT_TRUE(dut_.runtime().RunWithTimeoutOrUntil(
      [&]() {
        bool queued = false;
        dut_.RunInDriverContext(
            [&](Dwc3& drv) { queued = !Dwc3TestHelper::IsSharedFifoEmpty(drv); });
        return queued;
      },
      zx::sec(10)));

  dut_.RunInDriverContext([&](Dwc3& drv) {
    EXPECT_EQ(Dwc3TestHelper::GetEp0State(drv), Dwc3TestHelper::State::DataIn);
    Dwc3TestHelper::SimulateDataInPhase(drv);
    EXPECT_EQ(Dwc3TestHelper::GetEp0State(drv), Dwc3TestHelper::State::WaitNrdyOut);
    Dwc3TestHelper::SimulateStatusPhase(drv, false);
    EXPECT_EQ(Dwc3TestHelper::GetEp0State(drv), Dwc3TestHelper::State::Setup);
  });

  if (binding.has_value()) {
    binding->Unbind();
    dut_.runtime().RunUntilIdle();
  }

  TearDownAndPowerOffDriver();
}

TEST_F(UnmanagedTestFixture, DISABLED_ControlReadOversizedStallIn) {
  SetUpAndPowerOnDriver();

  FakeUsbDciInterface fake_dci;
  libsync::Completion control_called;
  fake_dci.SetControlCallback([&](fuchsia_hardware_usb_descriptor::wire::UsbSetup setup,
                                  cpp20::span<const uint8_t> data) { control_called.Signal(); });

  std::vector<uint8_t> read_data(65, 0xAA);
  fake_dci.SetReadData(std::move(read_data));

  auto binding = BindDciInterface(&fake_dci);

  dut_.RunInDriverContext([&](Dwc3& drv) {
    Dwc3TestHelper::SetEp0OutEnabled(drv, true);
    Dwc3TestHelper::SetEp0InEnabled(drv, true);
    Dwc3TestHelper::Ep0QueueSetup(drv);
    Dwc3TestHelper::SetEp0State(drv, Dwc3TestHelper::State::Setup);
    Dwc3TestHelper::SetControllerStarted(drv, true);

    auto setup = MakeGetDescriptorSetup(64);
    setup.bm_request_type = USB_DIR_IN | USB_TYPE_VENDOR | USB_RECIP_DEVICE;
    setup.b_request = 0x01;
    Dwc3TestHelper::SimulateSetupReceived(drv, setup);
  });

  EXPECT_TRUE(dut_.runtime().RunWithTimeoutOrUntil(
      [&]() {
        bool stalled = false;
        dut_.RunInDriverContext([&](Dwc3& drv) {
          stalled = (Dwc3TestHelper::GetEp0State(drv) == Dwc3TestHelper::State::Setup);
        });
        return stalled;
      },
      zx::sec(5)));

  if (binding.has_value()) {
    binding->Unbind();
    dut_.runtime().RunUntilIdle();
  }

  TearDownAndPowerOffDriver();
}

TEST_F(UnmanagedTestFixture, DISABLED_ControlWriteComplete) {
  SetUpAndPowerOnDriver();

  FakeUsbDciInterface fake_dci;
  libsync::Completion control_called;
  std::vector<uint8_t> received_data;
  std::atomic<size_t> callback_received_len{0};

  fake_dci.SetControlCallback(
      [&](fuchsia_hardware_usb_descriptor::wire::UsbSetup setup, cpp20::span<const uint8_t> data) {
        received_data.assign(data.begin(), data.end());
        callback_received_len.store(data.size());
        control_called.Signal();
      });

  auto binding = BindDciInterface(&fake_dci);

  dut_.RunInDriverContext([&](Dwc3& drv) {
    Dwc3TestHelper::SetEp0OutEnabled(drv, true);
    Dwc3TestHelper::SetEp0InEnabled(drv, true);
    Dwc3TestHelper::Ep0QueueSetup(drv);
    Dwc3TestHelper::SetEp0State(drv, Dwc3TestHelper::State::Setup);
    Dwc3TestHelper::SetControllerStarted(drv, true);

    auto setup = MakeSetupPacket(USB_DIR_OUT | USB_TYPE_VENDOR | USB_RECIP_DEVICE, 0x99, 0, 0, 8);
    void* buf = Dwc3TestHelper::GetEp0BufferVirt(drv);
    std::memcpy(buf, &setup, sizeof(setup));
    Dwc3TestHelper::HandleEp0TransferCompleteEvent(drv, 0);
    Dwc3TestHelper::ClearSharedFifo(drv);
  });

  ASSERT_EQ(ZX_OK, control_called.Wait(zx::sec(5)));

  if (binding.has_value()) {
    binding->Unbind();
    dut_.runtime().RunUntilIdle();
  }

  TearDownAndPowerOffDriver();
}

TEST_F(UnmanagedTestFixture, DISABLED_ControlWriteDataOutOverflow) {
  SetUpAndPowerOnDriver();

  FakeUsbDciInterface fake_dci;
  libsync::Completion control_called;
  std::vector<uint8_t> received_data;
  std::atomic<size_t> callback_received_len{0};

  fake_dci.SetControlCallback(
      [&](fuchsia_hardware_usb_descriptor::wire::UsbSetup setup, cpp20::span<const uint8_t> data) {
        received_data.assign(data.begin(), data.end());
        callback_received_len.store(data.size());
        control_called.Signal();
      });

  auto binding = BindDciInterface(&fake_dci);

  dut_.RunInDriverContext([&](Dwc3& drv) {
    Dwc3TestHelper::SetEp0OutEnabled(drv, true);
    Dwc3TestHelper::SetEp0InEnabled(drv, true);
    Dwc3TestHelper::Ep0QueueSetup(drv);
    Dwc3TestHelper::SetEp0State(drv, Dwc3TestHelper::State::Setup);
    Dwc3TestHelper::SetControllerStarted(drv, true);

    auto setup = MakeSetupPacket(USB_DIR_OUT | USB_TYPE_VENDOR | USB_RECIP_DEVICE, 0x01, 0, 0, 8);
    void* buf = Dwc3TestHelper::GetEp0BufferVirt(drv);
    std::memcpy(buf, &setup, sizeof(setup));
    Dwc3TestHelper::HandleEp0TransferCompleteEvent(drv, 0);
    Dwc3TestHelper::ClearSharedFifo(drv);
  });

  dut_.RunInDriverContext([&](Dwc3& drv) {
    EXPECT_EQ(Dwc3TestHelper::GetEp0State(drv), Dwc3TestHelper::State::DataOut);

    dwc3_trb_t trb{};
    trb.status = TRB_BUFSIZ(static_cast<uint32_t>(65536 - 16));
    Dwc3TestHelper::PushTrbToSharedFifo(drv, trb);

    Dwc3TestHelper::HandleEp0TransferCompleteEvent(drv, 0);

    EXPECT_TRUE(Dwc3TestHelper::IsEp0OutStalled(drv));
  });

  if (binding.has_value()) {
    binding->Unbind();
    dut_.runtime().RunUntilIdle();
  }

  TearDownAndPowerOffDriver();
}

TEST_F(UnmanagedTestFixture, ControlWriteShortPacketDataOut) {
  SetUpAndPowerOnDriver();

  FakeUsbDciInterface fake_dci;
  libsync::Completion control_called;
  std::vector<uint8_t> received_data;
  std::atomic<size_t> callback_received_len{0};

  fake_dci.SetControlCallback(
      [&](fuchsia_hardware_usb_descriptor::wire::UsbSetup setup, cpp20::span<const uint8_t> data) {
        received_data.assign(data.begin(), data.end());
        callback_received_len.store(data.size());
        control_called.Signal();
      });

  auto binding = BindDciInterface(&fake_dci);

  dut_.RunInDriverContext([&](Dwc3& drv) {
    Dwc3TestHelper::SetEp0OutEnabled(drv, true);
    Dwc3TestHelper::SetEp0InEnabled(drv, true);
    Dwc3TestHelper::Ep0QueueSetup(drv);
    Dwc3TestHelper::SetEp0State(drv, Dwc3TestHelper::State::Setup);
    Dwc3TestHelper::SetControllerStarted(drv, true);

    auto setup = MakeSetupPacket(USB_DIR_OUT | USB_TYPE_VENDOR | USB_RECIP_DEVICE, 0x01, 0, 0, 16);
    void* buf = Dwc3TestHelper::GetEp0BufferVirt(drv);
    std::memcpy(buf, &setup, sizeof(setup));
    Dwc3TestHelper::HandleEp0TransferCompleteEvent(drv, 0);
    Dwc3TestHelper::ClearSharedFifo(drv);
  });

  dut_.RunInDriverContext([&](Dwc3& drv) {
    EXPECT_EQ(Dwc3TestHelper::GetEp0State(drv), Dwc3TestHelper::State::DataOut);

    dwc3_trb_t trb{};
    trb.status = TRB_BUFSIZ(static_cast<uint32_t>(65536 - 8));
    Dwc3TestHelper::PushTrbToSharedFifo(drv, trb);

    Dwc3TestHelper::HandleEp0TransferCompleteEvent(drv, 0);
  });

  // Wait for callback to be executed deterministically
  EXPECT_TRUE(dut_.runtime().RunWithTimeoutOrUntil(
      [&]() { return callback_received_len.load() != 0; }, zx::sec(10)));
  EXPECT_EQ(callback_received_len.load(), 8u);

  if (binding.has_value()) {
    binding->Unbind();
    dut_.runtime().RunUntilIdle();
  }

  TearDownAndPowerOffDriver();
}

// TODO(b/509735595): Re-enable once the production fixes are landed.
TEST_F(UnmanagedTestFixture, DISABLED_MaxBufferSizeTransferIn) {
  FakeUsbDciInterface fake_dci;
  SetUpAndPowerOnDriver();

  std::optional<fidl::ServerBindingRef<fuchsia_hardware_usb_dci::UsbDciInterface>> binding;

  // Simulate device returning 65535 bytes
  std::vector<uint8_t> read_data(65535, 0xCC);
  fake_dci.SetReadData(read_data);

  binding = BindDciInterface(&fake_dci);

  dut_.RunInDriverContext([&](Dwc3& drv) {
    Dwc3TestHelper::SetControllerStarted(drv, true);
    Dwc3TestHelper::Ep0QueueSetup(drv);
    Dwc3TestHelper::SetEp0State(drv, Dwc3TestHelper::State::Setup);

    // Setup packet with wLength = 65535
    fuchsia_hardware_usb_descriptor::wire::UsbSetup setup;
    setup.bm_request_type = USB_DIR_IN | USB_TYPE_VENDOR | USB_RECIP_DEVICE;
    setup.b_request = 0x01;
    setup.w_length = 65535;

    // Use high-level primitive to simulate Setup received!
    Dwc3TestHelper::SimulateSetupReceived(drv, setup);
  });

  dut_.RunInDriverContext([&](Dwc3& drv) {
    // With the FIDL limit fix, oversized requests are stalled and state returns to Setup.
    EXPECT_EQ(Dwc3TestHelper::GetEp0State(drv), Dwc3TestHelper::State::Setup);
  });

  if (binding.has_value()) {
    binding->Unbind();
    dut_.runtime().RunUntilIdle();
  }

  TearDownAndPowerOffDriver();
}

TEST_F(UnmanagedTestFixture, DISABLED_ZlpOutTransferRequired) {
  FakeUsbDciInterface fake_dci;
  SetUpAndPowerOnDriver();

  std::optional<fidl::ServerBindingRef<fuchsia_hardware_usb_dci::UsbDciInterface>> binding;
  std::atomic<int> control_call_count = 0;
  fake_dci.SetControlCallback([&](fuchsia_hardware_usb_descriptor::wire::UsbSetup setup,
                                  cpp20::span<const uint8_t> data) { control_call_count++; });

  binding = BindDciInterface(&fake_dci);

  dut_.RunInDriverContext([&](Dwc3& drv) {
    Dwc3TestHelper::SetControllerStarted(drv, true);
    Dwc3TestHelper::Ep0QueueSetup(drv);
    Dwc3TestHelper::SetEp0State(drv, Dwc3TestHelper::State::Setup);

    // Setup packet with wLength = 512 (multiple of MPS)
    fuchsia_hardware_usb_descriptor::wire::UsbSetup setup;
    setup.bm_request_type = USB_DIR_OUT | USB_TYPE_VENDOR | USB_RECIP_DEVICE;
    setup.b_request = 0x01;
    setup.w_length = 512;

    // Use high-level primitive to simulate Setup received!
    Dwc3TestHelper::SimulateSetupReceived(drv, setup);
  });

  dut_.RunInDriverContext([&](Dwc3& drv) {
    // Simulate Data OUT phase completion with 512 bytes!
    Dwc3TestHelper::SimulateDataOutPhase(drv, 512);
  });

  dut_.RunInDriverContext([&](Dwc3& drv) {
    // Driver should have read 512 bytes!
    // Verify that the driver actually queued a TRB for the ZLP!
    ASSERT_FALSE(Dwc3TestHelper::IsSharedFifoEmpty(drv));

    // State should now be WaitNrdyIn!
    EXPECT_EQ(Dwc3TestHelper::GetEp0State(drv), Dwc3TestHelper::State::WaitNrdyIn);
  });

  // Wait for callback to be executed deterministically
  dut_.runtime().RunUntil([&]() { return control_call_count.load() != 0; });

  // Expect 1 call to Control (for the 64 bytes - wait, the driver chunks it but the test just
  // counts the callback)
  EXPECT_EQ(control_call_count.load(), 1);

  if (binding.has_value()) {
    binding->Unbind();
    dut_.runtime().RunUntilIdle();
  }

  TearDownAndPowerOffDriver();
}

}  // namespace dwc3
