// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! Utilities for embedding SSH keys into ZBI images.

use fho::{Result, bug};
use std::fs;
use std::path::Path;

/// Bootloader file name used by bootsvc to persist authorized keys into `/data/ssh/authorized_keys`.
pub const SSH_BOOTLOADER_FILE_NAME: &str = "ssh.authorized_keys";

/// Prepare the SSH key as boot loader file.
pub fn authorized_keys_to_boot_loader_file(src: &Path, dst: &Path) -> Result<()> {
    let mut v = Vec::new();
    let name = SSH_BOOTLOADER_FILE_NAME;
    let authorized_keys = fs::read(src).map_err(|e| bug!("{e}"))?;

    // The format for the boot loader files is described in
    // https://cs.opensource.google/fuchsia/fuchsia/+/main:sdk/lib/zbi-format/include/lib/zbi-format/zbi.h;l=229-237;drc=64cdcbf06860ab1f19b85b3c221debcadcae3b5d
    v.push(name.len().try_into().map_err(|_| {
        bug!("Invalid length for boot file name: {} cannot be converted to u8", name.len())
    })?);
    v.extend(name.as_bytes());
    v.extend(authorized_keys);

    fs::write(dst, v).map_err(|e| bug!("{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[fuchsia::test]
    fn test_authorized_keys_to_boot_loader_file() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("authorized_keys");
        let dst = dir.path().join("bootloader_file");

        std::fs::write(&src, b"ssh-ed25519 AAAAC3... test key").unwrap();
        authorized_keys_to_boot_loader_file(&src, &dst).unwrap();

        let bytes = std::fs::read(&dst).unwrap();
        assert_eq!(bytes[0], SSH_BOOTLOADER_FILE_NAME.len() as u8);
        assert_eq!(
            &bytes[1..1 + SSH_BOOTLOADER_FILE_NAME.len()],
            SSH_BOOTLOADER_FILE_NAME.as_bytes()
        );
        assert_eq!(&bytes[1 + SSH_BOOTLOADER_FILE_NAME.len()..], b"ssh-ed25519 AAAAC3... test key");
    }
}
