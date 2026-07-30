// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use core::str::Utf8Error;

use bitrs as _;
use regio::x86::{Cpuid, CpuidResult};

//
pub const MAX_LEAF_AND_VENDOR_STRING: Cpuid<0x0, 0x0, u32, u32, u32, u32> = Cpuid::new();

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
pub fn max_leaf() -> u32 {
    MAX_LEAF_AND_VENDOR_STRING.read().eax
}

/// A vendor string derived from CPUID.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VendorString([u8; 12]);

impl VendorString {
    /// Intel's vendor string.
    pub const INTEL: Self = Self(*b"GenuineIntel");

    /// AMD's vendor string.
    pub const AMD: Self = Self(*b"AuthenticAMD");

    /// Returns the processor's vendor string.
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    pub fn get() -> Self {
        Self::from_cpuid(MAX_LEAF_AND_VENDOR_STRING.read())
    }

    /// Returns the vendor string from leaf 0.
    pub fn from_cpuid(leaf0: CpuidResult<u32, u32, u32, u32>) -> Self {
        let CpuidResult { ebx, ecx, edx, .. } = leaf0;
        let ebx = ebx.to_le_bytes();
        let ecx = ecx.to_le_bytes();
        let edx = edx.to_le_bytes();
        Self([
            ebx[0], ebx[1], ebx[2], ebx[3], //
            edx[0], edx[1], edx[2], edx[3], //
            ecx[0], ecx[1], ecx[2], ecx[3], //
        ])
    }

    pub fn as_str(&self) -> Result<&str, Utf8Error> {
        str::from_utf8(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Basic exercise of the above utilities.
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    #[test]
    fn feature_detection() {
        let vendor = VendorString::get();
        println!("Vendor string: {}", vendor.as_str().unwrap_or("invalid vendor string"));
    }

    #[test]
    fn amd_vendor_string() {
        // EBX, ECX, EDX copied verbatim from the manual.
        let leaf0 = CpuidResult { eax: 0x7, ebx: 0x6874_7541, ecx: 0x444d_4163, edx: 0x6974_6e65 };
        let vendor_str = VendorString::from_cpuid(leaf0);
        assert_eq!(vendor_str, VendorString::AMD);
    }
}
