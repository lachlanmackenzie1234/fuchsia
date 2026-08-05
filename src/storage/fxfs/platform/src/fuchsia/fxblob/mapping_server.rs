// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::fuchsia::fxblob::directory::BlobDirectory;
use crate::fuchsia::pager::PagerBacked;
use anyhow::{Error, anyhow};
use fuchsia_merkle::Hash;
use futures::lock::Mutex;
use mapping::{Extents, MappingCommand, RawMappingCommand};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use vmo_fifo::AsyncSender;

// The `vmo-fifo` divides the VMO into two regions: a fixed-size command slots region, and a
// dynamically allocated payload region where the actual extents are written.
//
// The following is the layout for a 512KB VMO with 256 capacity:
// [ Headers (64B) | Command Slots: 256 * 24B = 6,144B | .. Padding to 8KB .. | Payload (504KB) ]
// Note: Each command slot takes 24 bytes because `RawMappingCommand` has six 4-byte fields.
//
// 504KB / 8-bytes per extent = 64,512 maximum extents bounded by the payload block.
const MAPPING_VMO_SIZE: u64 = 512 * 1024;

// With a maximum capacity of 256 pending mapping commands, this allows for an average of ~252
// extents per blob. In the worst case of maximum fragmentation (every 4KB block maps to one
// extent), 64,512 extents can map up to ~252MB of blob data (or ~504MB if block size is 8KB).
const PENDING_COMMANDS_CAPACITY: u32 = 256;

/// This maintains state for mappings between the driver paging system and Fxfs.
///
/// Note: It is the responsibility of the client to track concurrent attempts to open files and
/// broker them properly.
pub struct BlobMappingServer {
    blob_directory: Arc<BlobDirectory>,
    sender: Mutex<AsyncSender<RawMappingCommand>>,
    next_key: AtomicU64,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SessionState {
    /// The unique identifier for the session.
    pub key: u64,
    /// The uncompressed byte size of the blob.
    pub size: u64,
}

impl BlobMappingServer {
    pub fn new(blob_directory: Arc<BlobDirectory>) -> Result<Self, Error> {
        let vmo = zx::Vmo::create(MAPPING_VMO_SIZE)
            .map_err(|s| anyhow!("Failed to create VMO: {}", s))?;

        // We allow up to 256 pending commands. This is a guess - we may have to adjust this.
        let sender = AsyncSender::<RawMappingCommand>::new(
            vmo,
            8,                         // alignment
            PENDING_COMMANDS_CAPACITY, // capacity of commands
        )
        .map_err(|s| anyhow!("Failed to create Sender: {}", s))?;

        Ok(Self { blob_directory, sender: Mutex::new(sender), next_key: AtomicU64::new(1) })
    }

    /// Returns a duplicated handle to the mapping VMO.
    pub async fn clone_mapping(&self) -> Result<zx::Vmo, zx::Status> {
        let sender = self.sender.lock().await;
        sender.vmo().duplicate_handle(zx::Rights::SAME_RIGHTS)
    }

    /// Retrieves the extent mappings for the blob and registers a new mapping session. Returns a
    // `SessionState` containing the node size and the uniquely generated session key.
    pub async fn create_session(&self, hash: Hash) -> Result<SessionState, Error> {
        let node = self
            .blob_directory
            .open_blob(&hash.into())
            .await?
            .ok_or_else(|| anyhow!("Blob not found"))?;

        let extents = node.get_mapping_extents().await?;
        let size = node.as_ref().byte_size();
        let blob_count = extents.data.len() as u32;
        let metadata_count = extents.merkle.len() as u32;

        let key = self.next_key.fetch_add(1, Ordering::Relaxed);
        let allocation_size = (blob_count + metadata_count) as usize * std::mem::size_of::<u64>();

        if allocation_size > 0 {
            let mut sender = self.sender.lock().await;

            let mut payload = sender.reserve_payload(allocation_size).await?;
            let offset_in_vmo = payload.offset();

            for (mut chunk, val_res) in payload.data().chunks_mut(std::mem::size_of::<u64>()).zip(
                Extents::encode_extents_iter(&extents.data)
                    .chain(Extents::encode_extents_iter(&extents.merkle)),
            ) {
                chunk.write(val_res.to_le());
            }

            let command = MappingCommand::Mappings {
                key,
                offset: offset_in_vmo as u32,
                metadata_count,
                blob_count,
            };

            payload.commit(command.into()).await?;
        }

        Ok(SessionState { key, size })
    }

    /// Unregisters the blob mapping and signals the block driver to terminate tracking.
    pub async fn close_session(&self, session_key: u64) -> Result<(), Error> {
        let mut sender = self.sender.lock().await;
        sender.push(MappingCommand::CloseBlob { key: session_key }.into()).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fuchsia::fxblob::testing::{BlobFixture, new_blob_fixture};
    use delivery_blob::CompressionMode;
    use fuchsia_async as fasync;
    use vmo_fifo::Receiver;

    #[fuchsia::test]
    async fn test_blob_mapping_server() {
        let fixture = new_blob_fixture().await;
        // Test with a large amount of non-compressible data to generate many extents
        let data = vec![42; 300_000];
        let hash = fixture.write_blob(&data, CompressionMode::Never).await;

        let blob_dir = fixture
            .volume()
            .root()
            .clone()
            .as_node()
            .into_any()
            .downcast::<BlobDirectory>()
            .expect("Failed to downcast root directory to BlobDirectory");

        let node = blob_dir
            .open_blob(&hash.into())
            .await
            .expect("Failed to open blob in Fxfs")
            .expect("open_blob returned None instead of node");
        let extents = node.get_mapping_extents().await.expect("Failed to retrieve extents");
        let data_extents = extents.data;
        let merkle_extents = extents.merkle;

        // Un-dropped nodes pin the Blob as actively opened. The unmount routine in fixture.close()
        // will wait forever for this blob to be fully closed, causing a test timeout.
        drop(node);

        let server = BlobMappingServer::new(blob_dir).expect("Failed to create BlobMappingServer");
        let client_mapping = server.clone_mapping().await.expect("Failed to clone VMO mapping");

        let receiver_task = fasync::unblock(move || {
            let mut receiver = Receiver::<RawMappingCommand>::new(client_mapping, 256)
                .expect("Failed to create the Receiver wrapper");

            let pop_cmd = |receiver: &mut Receiver<RawMappingCommand>| loop {
                match receiver.pop_reserve() {
                    Ok(cmd) => break cmd,
                    Err(zx::Status::SHOULD_WAIT) => {
                        std::thread::sleep(std::time::Duration::from_millis(10))
                    }
                    Err(e) => panic!("pop_reserve failed: {:?}", e),
                }
            };

            // First Open Command
            let cmd1_raw = pop_cmd(&mut receiver);
            let cmd1 =
                MappingCommand::try_from(cmd1_raw).expect("Failed to convert raw mapping command");

            let (cmd1_offset, cmd1_blob_count, cmd1_metadata_count) = match cmd1 {
                MappingCommand::Mappings { key, offset, metadata_count, blob_count } => {
                    assert_eq!(key, 1);
                    assert_eq!(blob_count, data_extents.len() as u32);
                    assert_eq!(metadata_count, merkle_extents.len() as u32);
                    (offset, blob_count, metadata_count)
                }
                _ => panic!("Expected Mappings command"),
            };

            // Verify payload
            let total_extents = cmd1_blob_count + cmd1_metadata_count;
            let buffer = receiver.payload_slice(cmd1_offset, total_extents * 8).to_vec();

            let mut expected_payload = Vec::new();
            for val in Extents::encode_extents_iter(&data_extents)
                .chain(Extents::encode_extents_iter(&merkle_extents))
            {
                expected_payload.extend_from_slice(&val.to_le_bytes());
            }
            assert_eq!(buffer, expected_payload);
            receiver.pop_commit().expect("Failed pop_commit");

            let cmd2_raw = pop_cmd(&mut receiver);
            let cmd2 =
                MappingCommand::try_from(cmd2_raw).expect("Failed to convert raw mapping command");
            match cmd2 {
                MappingCommand::CloseBlob { key } => assert_eq!(key, 1),
                _ => panic!("Expected CloseBlob command"),
            };
            receiver.pop_commit().expect("Failed pop_commit");
        });

        let server_task = async move {
            let SessionState { key, .. } =
                server.create_session(hash).await.expect("create_session failed");
            assert_eq!(key, 1);

            server.close_session(key).await.expect("close_session failed on existing key");

            std::mem::drop(server);
        };

        futures::join!(receiver_task, server_task);

        fixture.close().await;
    }

    #[fuchsia::test]
    async fn test_missing_blob() {
        let fixture = new_blob_fixture().await;
        let blob_dir = fixture
            .volume()
            .root()
            .clone()
            .as_node()
            .into_any()
            .downcast::<BlobDirectory>()
            .expect("Failed to downcast");

        let server = BlobMappingServer::new(blob_dir).expect("Failed to create server");
        let hash = Hash::from([1u8; 32]);
        server
            .create_session(hash)
            .await
            .expect_err("create_session should fail with blob that doesn't exist");

        std::mem::drop(server);
        fixture.close().await;
    }

    #[fuchsia::test]
    async fn test_invalid_key() {
        let fixture = new_blob_fixture().await;
        let blob_dir = fixture
            .volume()
            .root()
            .clone()
            .as_node()
            .into_any()
            .downcast::<BlobDirectory>()
            .expect("Failed to downcast");

        let server = BlobMappingServer::new(blob_dir).expect("Failed to create server");
        server.close_session(42).await.expect("close_session should return Ok with invalid key");
        std::mem::drop(server);

        fixture.close().await;
    }
}
