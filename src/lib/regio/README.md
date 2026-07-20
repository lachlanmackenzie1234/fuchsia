# regio

`regio` ("reh-jee-o") is a general, extensible framework for register I/O. It is
agnostic of the choice of layout type, and aims to support varied notions of
'register' across common CPU architectures and hardware.

## `Register`

`Register` represents an abstract register tied to a specific layout, access
permissions, and an 'I/O backend' defining how it is accessed. Its API admits
the usual access methods when its permissions and backend permit them.

Conventions:

- It is expected that hardware-specific notions of register define themselves as
  a type alias or wrapper of `Register` and implement their own specialized
  constructors.

- When register instances are `const`-constructible, they ought to be `const`s
  and carry the same name as they are documented in architectural manuals. They
  are usually documented in screaming snake case already, so this makes for a
  nice, readable coincidence.

## `Value`

- `Value` represents a prospective value for a register. It is a thin wrapper of
  a layout value that retains its knowledge of the I/O backend, admitting
  mirrored writeback methods when the access permissions and backend permit
  them. Instances are returned by `Register::read()` and may also be minted via
  `Register::into_value()`.

## MMIO

The core MMIO types are `Offset`, `Mmio`, `MmioPtr`, and `MmioBank`. `Mmio` is
an alias of `Register` representing an MMIO register. Often MMIO addresses are
not statically known; in this case, we port the above `const` `Register`
convention over to defining the offset descriptors - which are usually
statically known - to be `const` instances bearing the documented register
names.

The following is example logic for driving a SiFive UART.

```rust
use core::ptr;

use bitrs::layout;
use regio::{MmioBank, MmioPtr, Offset, RwSafe};

layout!({
    struct TransmitDataRegister(u32);
    {
        let full @ 31; // Whether the TX FIFO is full.
        let __ @ 30..8;
        let data @ 7..0;
    }
});

const TXDATA: Offset<TransmitDataRegister, RwSafe> = Offset::new(0);

layout!({
    struct ReceiveDataRegister(u32);
    {
        let empty @ 31; // Whether the RX FIFO is empty.
        let __ @ 30..8;
        let data @ 7..0;
    }
});

const RXDATA: Offset<ReceiveDataRegister, RwSafe> = Offset::new(4);

layout!({
    struct InterruptEnableRegister(u32);
    {
        let _ @ 31..2;
        let rxwm @ 1; // Receive watermark interrupt enable
        let txwm @ 0; // Transmit watermark interrupt enable
    }
});

const IE: Offset<InterruptEnableRegister, RwSafe> = Offset::new(0x10);

// ...

const BANK_SIZE = 0x1c; // The highest documented offset is 0x18.

// Could be made const in this case, but in practice it may not be.
let uart_base = unsafe {
    MmioPtr::<u32, RwSafe>::new(ptr::with_exposed_provenance_mut(0x1001_3000))
};
let uart = MmioBank::new(uart_base, BANK_SIZE);

// Can access the MmioBank to get an Mmio.
unsafe { uart.at(IE) }.write(0u32.into());  // Disable all interrupts.

// Can construct a Value directly for write-back.
let data = TransmitDataRegister::from(0).set_data(0xab);
unsafe { uart.at(TXDATA) }.into_value(data).write();

// Simple reads can fluidly chain from MmioPtr -> Mmio -> Layout.
let _: u8 = unsafe { uart.at(RXDATA) }.read().data();
```

## x86 MSRs

`Msr` is an alias of `Register` used to model MSRs:

```rust
use regio::RwSafe;
use regio::x86::Msr;

const IA32_TIME_STAMP_COUNTER: Msr<0x10, u64, RwSafe> = Msr::new();

println!("Current timestamp: {:#x}", IA32_TIME_STAMP_COUNTER.read().get());
```
