// Copyright 2025 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include <lib/uart/dw8250.h>
#include <lib/uart/mock.h>
#include <lib/uart/uart.h>

#include <zxtest/zxtest.h>

namespace {

using SimpleTestDriver = uart::KernelDriver<uart::dw8250::Driver, uart::mock::IoProvider,
                                            uart::UnsynchronizedPolicy, uart::mock::IrqProvider>;
constexpr zbi_dcfg_simple_t kTestConfig = {};

template <typename Mock>
void AppendInitSequence(Mock& mock) {
  mock
      // Init()
      .ExpectRead(uint32_t{0x103d32}, 0x3d)  // ComponentParameterRegister read
                                             // From real hardware:
                                             // 0x103d32 =
                                             //   256 byte fifo
                                             //   DMA_EXTRA
                                             //   UART_ADD_ENCODED_PARAMS
                                             //   SHADOW
                                             //   FIFO_STAT
                                             //   ADDITIONAL_FAT
                                             //   THRE_MODE
                                             //   AFCE_MODE
                                             //   APB_DATA_WIDTH = 2

      .ExpectWrite(uint32_t{0x80}, 1)  // InterruptEnableRegister write
      .ExpectWrite(uint32_t{0x17}, 2)  // FifoControlRegister write
      .ExpectWrite(uint32_t{0x03}, 4)  // ModemControlRegister write
      // Record the divisor.
      .ExpectRead(uint32_t{0x03}, 3)   // LineControl (8N1)
      .ExpectWrite(uint32_t{0x83}, 3)  // LineControl (8N1 | DLAB)
      .ExpectRead(uint32_t{0x83}, 3)   // LineControl readback (took effect)
      .ExpectRead(uint32_t{0x0d}, 0)   // DivisorLatchLower
      .ExpectRead(uint32_t{0x01}, 1)   // DivisorLatchUpper
      .ExpectWrite(uint32_t{0x03}, 3)  // LineControl (8N1)
      .ExpectRead(uint32_t{0x03}, 3)   // LineControl readback (took effect)
      // End of Init()
      ;
}

// The sequence of register accesses made by PrepareForSuspend when the divisor
// has already been recorded.
template <typename Mock>
void AppendPrepareForSuspendSequence(Mock& mock) {
  mock.ExpectRead(uint32_t{0x85}, 1)   // InterruptEnable (PTIME | ELSI | ERBFI)
      .ExpectWrite(uint32_t{0x00}, 1)  // InterruptEnable (all disabled)
      .ExpectRead(uint32_t{0x03}, 4)   // ModemControl (RTS | DTR)
      .ExpectRead(uint32_t{0x03}, 3);  // LineControl (8N1)
}

// The sequence of register accesses made by WakeupFromSuspend after the state
// above has been saved (with a divisor of 0x010d), when every LCR write takes
// effect.
template <typename Mock>
void AppendWakeupFromSuspendSequence(Mock& mock) {
  mock.ExpectWrite(uint32_t{0x83}, 3)   // LineControl (8N1 | DLAB)
      .ExpectRead(uint32_t{0x83}, 3)    // LineControl readback (took effect)
      .ExpectWrite(uint32_t{0x0d}, 0)   // DivisorLatchLower
      .ExpectWrite(uint32_t{0x01}, 1)   // DivisorLatchUpper
      .ExpectWrite(uint32_t{0x03}, 3)   // LineControl (8N1)
      .ExpectRead(uint32_t{0x03}, 3)    // LineControl readback (took effect)
      .ExpectWrite(uint32_t{0x17}, 2)   // FifoControl
      .ExpectWrite(uint32_t{0x03}, 4)   // ModemControl
      .ExpectWrite(uint32_t{0x85}, 1);  // InterruptEnable
}

TEST(Dw8250Tests, HelloWorld) {
  SimpleTestDriver driver(kTestConfig);

  AppendInitSequence(driver.io().mock());
  driver.io()
      .mock()
      // Write()
      .ExpectRead(uint32_t{0b000'0010}, 31)  // UserStatus.TFNF (tx fifo not full)
      .ExpectRead(uint32_t{0x100}, 32)       // TransmitFifoLevel (reads 256)
      .ExpectWrite(uint32_t{'h'}, 0)         // Write
      .ExpectWrite(uint32_t{'i'}, 0)
      .ExpectWrite(uint32_t{'\r'}, 0)
      .ExpectWrite(uint32_t{'\n'}, 0);

  driver.Init();
  EXPECT_EQ(3, driver.Write("hi\n"));
}

TEST(Dw8250Tests, SetLineControl8N1) {
  SimpleTestDriver driver(kTestConfig);

  AppendInitSequence(driver.io().mock());
  driver.io()
      .mock()
      // SetLineControl()
      .ExpectRead(uint32_t{0b0000'0000}, 31)  // UserStatus
      .ExpectWrite(uint32_t{0b1000'0000}, 3)  // LineControl setting divisor latch access
      .ExpectWrite(uint32_t{0b0000'0001}, 0)  // Divisor to 1
      .ExpectWrite(uint32_t{0b0000'0000}, 1)
      .ExpectRead(uint32_t{0b0000'0000}, 31)   // UserStatus
      .ExpectWrite(uint32_t{0b0000'0011}, 3);  // LineControl setting 8N1

  driver.Init();
  driver.SetLineControl(uart::DataBits::k8, uart::Parity::kNone, uart::StopBits::k1);
}

TEST(Dw8250Tests, SetLineControl7E1) {
  SimpleTestDriver driver(kTestConfig);

  AppendInitSequence(driver.io().mock());
  driver.io()
      .mock()
      // SetLineControl()
      .ExpectRead(uint32_t{0b0000'0000}, 31)  // UserStatus
      .ExpectWrite(uint32_t{0b1000'0000}, 3)  // LineControl setting divisor latch access
      .ExpectWrite(uint32_t{0b0000'0001}, 0)  // Divisor to 1
      .ExpectWrite(uint32_t{0b0000'0000}, 1)
      .ExpectRead(uint32_t{0b0000'0000}, 31)   // UserStatus
      .ExpectWrite(uint32_t{0b0001'1010}, 3);  // LineControl setting 7E1

  driver.Init();
  driver.SetLineControl(uart::DataBits::k7, uart::Parity::kEven, uart::StopBits::k1);
}

TEST(Dw8250Tests, Write) {
  SimpleTestDriver driver(kTestConfig);

  // Write using the expected FIFO_STAT mode
  AppendInitSequence(driver.io().mock());
  driver.io()
      .mock()
      // Write()
      .ExpectRead(uint32_t{0b000'0000}, 31)  // UserStatus.TFNF (tx fifo full)
      // TX fifo is full, poll until it isn't.
      .ExpectRead(uint32_t{0b000'0010}, 31)  // UserStatus.TFNF (tx fifo not full)
      .ExpectRead(uint32_t{0x100 - 2}, 32)   // TransmitFifoLevel (reads 256 - 2)
      // Only 2 bytes available in the tx fifo.
      .ExpectWrite(uint32_t{'a'}, 0)  // Write
      .ExpectWrite(uint32_t{'b'}, 0)
      // Go back to polling if the fifo has space in it.
      .ExpectRead(uint32_t{0b000'0000}, 31)  // UserStatus.TFNF (tx fifo full)
      // TX fifo is full, poll until it isn't.
      .ExpectRead(uint32_t{0b000'0010}, 31)  // UserStatus.TFNF (tx fifo not full)
      .ExpectRead(uint32_t{0x100}, 32)       // TransmitFifoLevel (reads 256)
      // Write the rest of the message.
      .ExpectWrite(uint32_t{'c'}, 0)
      .ExpectWrite(uint32_t{'d'}, 0)
      .ExpectWrite(uint32_t{'e'}, 0)
      .ExpectWrite(uint32_t{'f'}, 0);

  driver.Init();
  EXPECT_EQ(6, driver.Write("abcdef"));
}

TEST(Dw8250Tests, Read) {
  SimpleTestDriver driver(kTestConfig);

  AppendInitSequence(driver.io().mock());
  driver.io()
      .mock()
      // Write()
      .ExpectRead(uint32_t{0b000'0010}, 31)  // UserStatus.TFNF (tx fifo not full)
      .ExpectRead(uint32_t{0x100}, 32)       // TransmitFifoLevel (reads 256)
      .ExpectWrite(uint32_t{'?'}, 0)         // Write
      .ExpectWrite(uint32_t{'\r'}, 0)
      .ExpectWrite(uint32_t{'\n'}, 0)
      // Read()
      .ExpectRead(uint32_t{0b0110'0001}, 5)  // Read (data_ready)
      .ExpectRead(uint32_t{'q'}, 0)          // Read (data)
      // Read()
      .ExpectRead(uint32_t{0b0110'0001}, 5)  // Read (data_ready)
      .ExpectRead(uint32_t{'\n'}, 0);        // Read (data)

  driver.Init();
  EXPECT_EQ(2, driver.Write("?\n"));
  EXPECT_EQ(uint32_t{'q'}, driver.Read());
  EXPECT_EQ(uint32_t{'\n'}, driver.Read());
}

TEST(Dw8250Tests, TxDrained) {
  SimpleTestDriver driver(kTestConfig);

  AppendInitSequence(driver.io().mock());
  driver.io()
      .mock()
      .ExpectRead(uint32_t{0b0010'0000}, 5)   // LineStatus (THRE set, TEMT clear)
      .ExpectRead(uint32_t{0b0110'0000}, 5);  // LineStatus (THRE and TEMT set)

  driver.Init();
  EXPECT_FALSE(driver.TxDrained());
  EXPECT_TRUE(driver.TxDrained());
}

TEST(Dw8250Tests, SuspendResume) {
  SimpleTestDriver driver(kTestConfig);

  auto& mock = driver.io().mock();
  AppendInitSequence(mock);
  // The divisor was recorded by Init, so PrepareForSuspend does not touch it.
  AppendPrepareForSuspendSequence(mock);
  AppendWakeupFromSuspendSequence(mock);

  driver.Init();
  driver.PrepareForSuspend();
  // A second suspend is a no-op.
  driver.PrepareForSuspend();
  driver.WakeupFromSuspend();
  // A second wakeup is a no-op.
  driver.WakeupFromSuspend();
}

// If an LCR write is ignored because the UART is busy, the driver must notice
// (by reading LCR back), force the UART idle, and try again.  In particular, it
// must not touch the divisor latch unless DLAB was actually set, since
// otherwise the "divisor" writes would land in THR and IER.
TEST(Dw8250Tests, WakeupRetriesIgnoredLcrWrite) {
  SimpleTestDriver driver(kTestConfig);

  auto& mock = driver.io().mock();
  AppendInitSequence(mock);
  AppendPrepareForSuspendSequence(mock);
  mock
      // WakeupFromSuspend()
      .ExpectWrite(uint32_t{0x83}, 3)   // LineControl (8N1 | DLAB)
      .ExpectRead(uint32_t{0x00}, 3)    // LineControl readback (ignored, UART was busy)
      .ExpectWrite(uint32_t{0x17}, 2)   // FifoControl (reset FIFOs to force idle)
      .ExpectRead(uint32_t{0x00}, 0)    // RxBuffer (drain)
      .ExpectWrite(uint32_t{0x83}, 3)   // LineControl (8N1 | DLAB)
      .ExpectRead(uint32_t{0x83}, 3)    // LineControl readback (took effect)
      .ExpectWrite(uint32_t{0x0d}, 0)   // DivisorLatchLower
      .ExpectWrite(uint32_t{0x01}, 1)   // DivisorLatchUpper
      .ExpectWrite(uint32_t{0x03}, 3)   // LineControl (8N1)
      .ExpectRead(uint32_t{0x83}, 3)    // LineControl readback (ignored, UART was busy)
      .ExpectWrite(uint32_t{0x17}, 2)   // FifoControl (reset FIFOs to force idle)
      .ExpectRead(uint32_t{0x00}, 0)    // DivisorLatchLower (drain, DLAB still set)
      .ExpectWrite(uint32_t{0x03}, 3)   // LineControl (8N1)
      .ExpectRead(uint32_t{0x03}, 3)    // LineControl readback (took effect)
      .ExpectWrite(uint32_t{0x17}, 2)   // FifoControl
      .ExpectWrite(uint32_t{0x03}, 4)   // ModemControl
      .ExpectWrite(uint32_t{0x85}, 1);  // InterruptEnable

  driver.Init();
  driver.PrepareForSuspend();
  driver.WakeupFromSuspend();
}

// If Init is unable to record the divisor (because LCR.DLAB could never be
// set), PrepareForSuspend tries again.
TEST(Dw8250Tests, DivisorRecordedBySuspendIfInitFailed) {
  SimpleTestDriver driver(kTestConfig);

  auto& mock = driver.io().mock();
  mock
      // Init()
      .ExpectRead(uint32_t{0x103d32}, 0x3d)  // ComponentParameterRegister read
      .ExpectWrite(uint32_t{0x80}, 1)        // InterruptEnableRegister write
      .ExpectWrite(uint32_t{0x17}, 2)        // FifoControlRegister write
      .ExpectWrite(uint32_t{0x03}, 4)        // ModemControlRegister write
      .ExpectRead(uint32_t{0x03}, 3);        // LineControl (8N1)
  // Every attempt to set DLAB is ignored.
  for (size_t i = 0; i < uart::dw8250::kLcrWriteAttempts; ++i) {
    mock.ExpectWrite(uint32_t{0x83}, 3)  // LineControl (8N1 | DLAB)
        .ExpectRead(uint32_t{0x03}, 3)   // LineControl readback (ignored)
        .ExpectWrite(uint32_t{0x17}, 2)  // FifoControl (reset FIFOs to force idle)
        .ExpectRead(uint32_t{0x00}, 0);  // RxBuffer (drain)
  }
  AppendPrepareForSuspendSequence(mock);
  mock
      // PrepareForSuspend() records the divisor.
      .ExpectWrite(uint32_t{0x83}, 3)  // LineControl (8N1 | DLAB)
      .ExpectRead(uint32_t{0x83}, 3)   // LineControl readback (took effect)
      .ExpectRead(uint32_t{0x0d}, 0)   // DivisorLatchLower
      .ExpectRead(uint32_t{0x01}, 1)   // DivisorLatchUpper
      .ExpectWrite(uint32_t{0x03}, 3)  // LineControl (8N1)
      .ExpectRead(uint32_t{0x03}, 3);  // LineControl readback (took effect)
  AppendWakeupFromSuspendSequence(mock);

  driver.Init();
  driver.PrepareForSuspend();
  driver.WakeupFromSuspend();
}

// If the divisor has never been recorded, WakeupFromSuspend leaves the divisor
// latch alone rather than programming garbage.
TEST(Dw8250Tests, WakeupWithoutDivisor) {
  SimpleTestDriver driver(kTestConfig);

  auto& mock = driver.io().mock();
  mock
      // Init()
      .ExpectRead(uint32_t{0x103d32}, 0x3d)  // ComponentParameterRegister read
      .ExpectWrite(uint32_t{0x80}, 1)        // InterruptEnableRegister write
      .ExpectWrite(uint32_t{0x17}, 2)        // FifoControlRegister write
      .ExpectWrite(uint32_t{0x03}, 4)        // ModemControlRegister write
      .ExpectRead(uint32_t{0x03}, 3);        // LineControl (8N1)
  for (size_t i = 0; i < uart::dw8250::kLcrWriteAttempts; ++i) {
    mock.ExpectWrite(uint32_t{0x83}, 3)  // LineControl (8N1 | DLAB)
        .ExpectRead(uint32_t{0x03}, 3)   // LineControl readback (ignored)
        .ExpectWrite(uint32_t{0x17}, 2)  // FifoControl (reset FIFOs to force idle)
        .ExpectRead(uint32_t{0x00}, 0);  // RxBuffer (drain)
  }
  AppendPrepareForSuspendSequence(mock);
  // PrepareForSuspend's attempt to record the divisor fails too.
  for (size_t i = 0; i < uart::dw8250::kLcrWriteAttempts; ++i) {
    mock.ExpectWrite(uint32_t{0x83}, 3)  // LineControl (8N1 | DLAB)
        .ExpectRead(uint32_t{0x03}, 3)   // LineControl readback (ignored)
        .ExpectWrite(uint32_t{0x17}, 2)  // FifoControl (reset FIFOs to force idle)
        .ExpectRead(uint32_t{0x00}, 0);  // RxBuffer (drain)
  }
  mock
      // WakeupFromSuspend()
      .ExpectWrite(uint32_t{0x03}, 3)   // LineControl (8N1)
      .ExpectRead(uint32_t{0x03}, 3)    // LineControl readback (took effect)
      .ExpectWrite(uint32_t{0x17}, 2)   // FifoControl
      .ExpectWrite(uint32_t{0x03}, 4)   // ModemControl
      .ExpectWrite(uint32_t{0x85}, 1);  // InterruptEnable

  driver.Init();
  driver.PrepareForSuspend();
  driver.WakeupFromSuspend();
}

TEST(Dw8250Tests, WakeupWithoutSuspendIsNoop) {
  SimpleTestDriver driver(kTestConfig);

  AppendInitSequence(driver.io().mock());

  driver.Init();
  driver.WakeupFromSuspend();
}

}  // namespace
