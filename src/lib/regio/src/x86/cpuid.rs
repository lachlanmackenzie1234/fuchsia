// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::{IoHandle, LayoutOver, Register, Ro};

/// The raw output of a CPUID instruction.
#[derive(Clone, Copy, Debug)]
pub struct CpuidRawResult {
    /// The output EAX register.
    pub eax: u32,
    /// The output EBX register.
    pub ebx: u32,
    /// The output ECX register.
    pub ecx: u32,
    /// The output EDX register.
    pub edx: u32,
}

/// The structured output of a CPUID instruction.
#[derive(Clone, Copy, Debug)]
pub struct CpuidResult<Eax, Ebx, Ecx, Edx>
where
    Eax: LayoutOver<u32>,
    Ebx: LayoutOver<u32>,
    Ecx: LayoutOver<u32>,
    Edx: LayoutOver<u32>,
{
    /// The output EAX register.
    pub eax: Eax,
    /// The output EBX register.
    pub ebx: Ebx,
    /// The output ECX register.
    pub ecx: Ecx,
    /// The output EDX register.
    pub edx: Edx,
}

//
// The following two implementations implicitly give CpuidResult an
// implementation of LayoutOver<CpuidRawResult>.
//

impl<Eax, Ebx, Ecx, Edx> From<CpuidResult<Eax, Ebx, Ecx, Edx>> for CpuidRawResult
where
    Eax: LayoutOver<u32>,
    Ebx: LayoutOver<u32>,
    Ecx: LayoutOver<u32>,
    Edx: LayoutOver<u32>,
{
    fn from(value: CpuidResult<Eax, Ebx, Ecx, Edx>) -> Self {
        Self {
            eax: value.eax.into(),
            ebx: value.ebx.into(),
            ecx: value.ecx.into(),
            edx: value.edx.into(),
        }
    }
}

impl<Eax, Ebx, Ecx, Edx> From<CpuidRawResult> for CpuidResult<Eax, Ebx, Ecx, Edx>
where
    Eax: LayoutOver<u32>,
    Ebx: LayoutOver<u32>,
    Ecx: LayoutOver<u32>,
    Edx: LayoutOver<u32>,
{
    fn from(value: CpuidRawResult) -> Self {
        Self {
            eax: value.eax.into(),
            ebx: value.ebx.into(),
            ecx: value.ecx.into(),
            edx: value.edx.into(),
        }
    }
}

/// Models a CPUID (sub)leaf, as identified by its LEAF and SUBLEAF identifiers.
/// It aliases [`Register`] with a layout type of [`CpuidResult`], and is of
/// course read-only.
///
/// Example usage:
/// ```
/// use regio::x86::Cpuid;
///
/// const CPUID_MAX_LEAF_AND_VENDOR_STRING: Cpuid<0x0, 0x0, u32, u32, u32, u32> = Cpuid::new();
///
/// println!("Maximum CPUID leaf number: {:#x}", CPUID_MAX_LEAF_AND_VENDOR_STRING.read().eax);
/// ```
pub type Cpuid<const LEAF: u32, const SUBLEAF: u32, Eax, Ebx, Ecx, Edx> =
    Register<CpuidResult<Eax, Ebx, Ecx, Edx>, Ro, CpuidIo<LEAF, SUBLEAF>>;

impl<const LEAF: u32, const SUBLEAF: u32, Eax, Ebx, Ecx, Edx>
    Cpuid<LEAF, SUBLEAF, Eax, Ebx, Ecx, Edx>
where
    Eax: LayoutOver<u32>,
    Ebx: LayoutOver<u32>,
    Ecx: LayoutOver<u32>,
    Edx: LayoutOver<u32>,
{
    /// Constructs a new CPUID leaf instance.
    pub const fn new() -> Self {
        // Safety: There's nothing unsafe about CpuidIo construction.
        unsafe { Register::from_io(CpuidIo {}) }
    }
}

/// A simple I/O backend for reading CPUID (sub)leaves.
pub struct CpuidIo<const LEAF: u32, const SUBLEAF: u32>;

impl<const LEAF: u32, const SUBLEAF: u32> IoHandle for CpuidIo<LEAF, SUBLEAF> {
    type Base = CpuidRawResult;
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod x86_only {
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::{__cpuid_count, CpuidResult as ArchCpuidResult};

    #[cfg(target_arch = "x86")]
    use core::arch::x86::{__cpuid_count, CpuidResult as ArchCpuidResult};

    use super::*;
    use crate::ReadHandle;

    impl<const LEAF: u32, const SUBLEAF: u32> ReadHandle for CpuidIo<LEAF, SUBLEAF> {
        #[inline]
        unsafe fn read_raw(&self) -> CpuidRawResult {
            let ArchCpuidResult { eax, ebx, ecx, edx } = __cpuid_count(LEAF, SUBLEAF);
            CpuidRawResult { eax, ebx, ecx, edx }
        }
    }
}

#[cfg(all(test, any(target_arch = "x86", target_arch = "x86_64")))]
mod tests {
    use super::*;

    // Little more than a simple compilation test of the intended usage pattern.
    #[test]
    fn cpuid() {
        const CPUID_MAX_LEAF_AND_VENDOR_STRING: Cpuid<0x0, 0x0, u32, u32, u32, u32> = Cpuid::new();
        println!("Maximum CPUID leaf number: {:#x}", CPUID_MAX_LEAF_AND_VENDOR_STRING.read().eax);
    }
}
