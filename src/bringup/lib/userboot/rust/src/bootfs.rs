// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use anyhow::{Error, anyhow};
use std::slice;
use zbi::zbi_format::ZBI_FLAGS_STORAGE_COMPRESSED;
use zbi::{ZbiContainer, ZbiType};
use zerocopy::IntoBytes as _;
use zx::{Name, VmarFlags, Vmo};

/// Maps the ZBI VMO into a VMAR and parses the ZBI container.
pub fn get_zbi_container(
    zbi_vmo: &Vmo,
    vmar: &zx::Vmar,
) -> Result<ZbiContainer<&'static [u8]>, Error> {
    let zbi_size = zbi_vmo.get_size()? as usize;
    let zbi_addr = vmar.map(0, zbi_vmo, 0, zbi_size, VmarFlags::PERM_READ)?;
    // SAFETY: zbi_addr points to a valid memory mapping in vmar of zbi_size bytes with read
    // permissions.
    let zbi_slice = unsafe {
        slice::from_raw_parts(core::ptr::with_exposed_provenance::<u8>(zbi_addr), zbi_size)
    };
    ZbiContainer::parse(zbi_slice).map_err(|e| anyhow!("Failed to parse ZBI: {e:?}"))
}

/// Extracts the BOOTFS VMO from the ZBI container, decompressing it if needed.
pub fn get_bootfs_vmo(container: &ZbiContainer<&[u8]>, vmar: &zx::Vmar) -> Result<Vmo, Error> {
    let bootfs_item = container
        .iter()
        .find(|item| item.header.type_ == ZbiType::StorageBootFs as u32)
        .ok_or_else(|| anyhow!("StorageBootFs item not found in ZBI"))?;

    let is_compressed = (bootfs_item.header.flags & ZBI_FLAGS_STORAGE_COMPRESSED) != 0;
    let bootfs_vmo = if is_compressed {
        let uncompressed_size = bootfs_item.header.extra as usize;
        let vmo = Vmo::create(uncompressed_size as u64)?;
        let bootfs_addr =
            vmar.map(0, &vmo, 0, uncompressed_size, VmarFlags::PERM_READ | VmarFlags::PERM_WRITE)?;

        let payload = bootfs_item.payload.as_bytes();
        // SAFETY: bootfs_addr points to a valid writable memory mapping in vmar of
        // uncompressed_size bytes.
        let dst_slice = unsafe {
            slice::from_raw_parts_mut(
                core::ptr::with_exposed_provenance_mut::<u8>(bootfs_addr),
                uncompressed_size,
            )
        };
        zstd_safe::decompress(dst_slice, payload)
            .map_err(|e| anyhow!("zstd decompression failed with error code: {}", e))?;

        // SAFETY: bootfs_addr was successfully mapped above with length uncompressed_size in vmar.
        unsafe {
            vmar.unmap(bootfs_addr, uncompressed_size)?;
        }
        vmo
    } else {
        let payload_bytes = bootfs_item.payload.as_bytes();
        let vmo = Vmo::create(payload_bytes.len() as u64)?;
        vmo.write(payload_bytes, 0)?;
        vmo
    };

    bootfs_vmo.set_name(&Name::new("uncompressed-bootfs")?)?;
    Ok(bootfs_vmo)
}
