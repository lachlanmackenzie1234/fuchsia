// Copyright 2019 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use fidl_fuchsia_io as fio;
use futures::future::BoxFuture;
use futures::prelude::*;
use log::error;
use omaha_client::storage::Storage;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

const STORAGE_FILE_PATH: &str = "omaha_client.json";
const STORAGE_TEMP_FILE_PATH: &str = "omaha_client_tmp.json";

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("while serializing state")]
    Serialize(#[source] serde_json::Error),

    #[error("while opening temporary file")]
    OpenTempFile(#[source] fuchsia_fs::node::OpenError),

    #[error("while writing temporary file")]
    WriteTempFile(#[source] fuchsia_fs::file::WriteError),

    #[error("while sending sync request for temporary file")]
    SyncTempFileFidl(#[source] fidl::Error),

    #[error("while syncing temporary file")]
    SyncTempFile(#[source] zx::Status),

    #[error("while closing temporary file")]
    CloseTempFile(#[source] fuchsia_fs::node::CloseError),

    #[error("while renaming temporary file to permanent file")]
    Rename(#[source] fuchsia_fs::node::RenameError),

    #[error("while sending sync request for storage directory")]
    SyncDirectoryFidl(#[source] fidl::Error),

    #[error("while syncing storage directory")]
    SyncDirectory(#[source] zx::Status),
}

#[derive(Debug, Error)]
enum LoadError {
    #[error("while opening storage file")]
    Open(#[source] fuchsia_fs::node::OpenError),

    #[error("while reading storage file")]
    Read(#[source] fuchsia_fs::file::ReadError),

    #[error("while deserializing storage state")]
    Deserialize(#[source] serde_json::Error),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StoredValue {
    String(String),
    Int(i64),
    Bool(bool),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StorageState {
    entries: BTreeMap<String, StoredValue>,
}

/// An implementation of the [`omaha_client::storage::Storage`] trait that persists state to a
/// JSON file using atomic rename.
pub struct AtomicStorage {
    storage_dir: fio::DirectoryProxy,
    state: StorageState,
}

impl AtomicStorage {
    /// Create a new `AtomicStorage` backed by the given directory proxy.
    pub async fn new(storage_dir: fio::DirectoryProxy) -> Self {
        let state = match Self::read_state(&storage_dir).await {
            Ok(state) => state,
            Err(e) => {
                error!(
                    "Failed to load storage state from {STORAGE_FILE_PATH}, falling back to default: {:#}",
                    anyhow::anyhow!(e)
                );
                StorageState::default()
            }
        };
        Self { storage_dir, state }
    }

    async fn read_state(storage_dir: &fio::DirectoryProxy) -> Result<StorageState, LoadError> {
        let file = match fuchsia_fs::directory::open_file(
            storage_dir,
            STORAGE_FILE_PATH,
            fio::PERM_READABLE,
        )
        .await
        {
            Ok(file) => file,
            Err(e) if e.is_not_found_error() => return Ok(StorageState::default()),
            Err(e) => return Err(LoadError::Open(e)),
        };
        let bytes = fuchsia_fs::file::read(&file).await.map_err(LoadError::Read)?;
        serde_json::from_slice(&bytes).map_err(LoadError::Deserialize)
    }
}

impl Storage for AtomicStorage {
    type Error = StorageError;

    fn get_string<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Option<String>> {
        let res = match self.state.entries.get(key) {
            Some(StoredValue::String(s)) => Some(s.clone()),
            _ => None,
        };
        future::ready(res).boxed()
    }

    fn get_int<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Option<i64>> {
        let res = match self.state.entries.get(key) {
            Some(StoredValue::Int(i)) => Some(*i),
            _ => None,
        };
        future::ready(res).boxed()
    }

    fn get_bool<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Option<bool>> {
        let res = match self.state.entries.get(key) {
            Some(StoredValue::Bool(b)) => Some(*b),
            _ => None,
        };
        future::ready(res).boxed()
    }

    fn set_string<'a>(
        &'a mut self,
        key: &'a str,
        value: &'a str,
    ) -> BoxFuture<'a, Result<(), Self::Error>> {
        self.state.entries.insert(key.to_string(), StoredValue::String(value.to_string()));
        future::ready(Ok(())).boxed()
    }

    fn set_int<'a>(
        &'a mut self,
        key: &'a str,
        value: i64,
    ) -> BoxFuture<'a, Result<(), Self::Error>> {
        self.state.entries.insert(key.to_string(), StoredValue::Int(value));
        future::ready(Ok(())).boxed()
    }

    fn set_bool<'a>(
        &'a mut self,
        key: &'a str,
        value: bool,
    ) -> BoxFuture<'a, Result<(), Self::Error>> {
        self.state.entries.insert(key.to_string(), StoredValue::Bool(value));
        future::ready(Ok(())).boxed()
    }

    fn remove<'a>(&'a mut self, key: &'a str) -> BoxFuture<'a, Result<(), Self::Error>> {
        self.state.entries.remove(key);
        future::ready(Ok(())).boxed()
    }

    fn commit(&mut self) -> BoxFuture<'_, Result<(), Self::Error>> {
        async move {
            let data = serde_json::to_vec(&self.state).map_err(StorageError::Serialize)?;

            let temp_file = fuchsia_fs::directory::open_file(
                &self.storage_dir,
                STORAGE_TEMP_FILE_PATH,
                fio::Flags::FLAG_MAYBE_CREATE | fio::Flags::FILE_TRUNCATE | fio::PERM_WRITABLE,
            )
            .await
            .map_err(StorageError::OpenTempFile)?;

            fuchsia_fs::file::write(&temp_file, &data)
                .await
                .map_err(StorageError::WriteTempFile)?;

            temp_file
                .sync()
                .await
                .map_err(StorageError::SyncTempFileFidl)?
                .map_err(zx::Status::err_from_raw)
                .or_else(
                    |status| {
                        if status == zx::Status::NOT_SUPPORTED { Ok(()) } else { Err(status) }
                    },
                )
                .map_err(StorageError::SyncTempFile)?;

            fuchsia_fs::file::close(temp_file).await.map_err(StorageError::CloseTempFile)?;

            fuchsia_fs::directory::rename(
                &self.storage_dir,
                STORAGE_TEMP_FILE_PATH,
                STORAGE_FILE_PATH,
            )
            .await
            .map_err(StorageError::Rename)?;

            self.storage_dir
                .sync()
                .await
                .map_err(StorageError::SyncDirectoryFidl)?
                .map_err(zx::Status::err_from_raw)
                .or_else(
                    |status| {
                        if status == zx::Status::NOT_SUPPORTED { Ok(()) } else { Err(status) }
                    },
                )
                .map_err(StorageError::SyncDirectory)?;

            Ok(())
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omaha_client::storage::tests::*;

    fn open_tempdir(tempdir: &tempfile::TempDir) -> fio::DirectoryProxy {
        fuchsia_fs::directory::open_in_namespace(
            tempdir.path().to_str().expect("tempdir path is not valid UTF-8"),
            fio::PERM_READABLE | fio::PERM_WRITABLE,
        )
        .expect("failed to open connection to tempdir")
    }

    #[fuchsia::test]
    async fn test_set_get_remove_string() {
        let tempdir = tempfile::tempdir().unwrap();
        let mut storage = AtomicStorage::new(open_tempdir(&tempdir)).await;
        do_test_set_get_remove_string(&mut storage).await;
    }

    #[fuchsia::test]
    async fn test_set_get_remove_int() {
        let tempdir = tempfile::tempdir().unwrap();
        let mut storage = AtomicStorage::new(open_tempdir(&tempdir)).await;
        do_test_set_get_remove_int(&mut storage).await;
    }

    #[fuchsia::test]
    async fn test_set_option_int() {
        let tempdir = tempfile::tempdir().unwrap();
        let mut storage = AtomicStorage::new(open_tempdir(&tempdir)).await;
        do_test_set_option_int(&mut storage).await;
    }

    #[fuchsia::test]
    async fn test_set_get_remove_bool() {
        let tempdir = tempfile::tempdir().unwrap();
        let mut storage = AtomicStorage::new(open_tempdir(&tempdir)).await;
        do_test_set_get_remove_bool(&mut storage).await;
    }

    #[fuchsia::test]
    async fn test_set_get_remove_time() {
        let tempdir = tempfile::tempdir().unwrap();
        let mut storage = AtomicStorage::new(open_tempdir(&tempdir)).await;
        do_test_set_get_remove_time(&mut storage).await;
    }

    #[fuchsia::test]
    async fn test_return_none_for_wrong_value_type() {
        let tempdir = tempfile::tempdir().unwrap();
        let mut storage = AtomicStorage::new(open_tempdir(&tempdir)).await;
        do_return_none_for_wrong_value_type(&mut storage).await;
    }

    #[fuchsia::test]
    async fn test_ensure_no_error_remove_nonexistent_key() {
        let tempdir = tempfile::tempdir().unwrap();
        let mut storage = AtomicStorage::new(open_tempdir(&tempdir)).await;
        do_ensure_no_error_remove_nonexistent_key(&mut storage).await;
    }

    #[fuchsia::test]
    async fn test_persistence_across_instances() {
        let tempdir = tempfile::tempdir().unwrap();
        {
            let mut storage = AtomicStorage::new(open_tempdir(&tempdir)).await;
            storage.set_string("channel", "stable").await.unwrap();
            storage.set_int("counter", 42).await.unwrap();
            // Before commit, not on disk
        }
        {
            let mut storage = AtomicStorage::new(open_tempdir(&tempdir)).await;
            assert_eq!(storage.get_string("channel").await, None);
            storage.set_string("channel", "beta").await.unwrap();
            storage.commit().await.unwrap();
            assert!(!tempdir.path().join(STORAGE_TEMP_FILE_PATH).exists());
            assert!(tempdir.path().join(STORAGE_FILE_PATH).exists());
        }
        {
            let storage = AtomicStorage::new(open_tempdir(&tempdir)).await;
            assert_eq!(storage.get_string("channel").await, Some("beta".to_string()));
        }
    }

    #[fuchsia::test]
    async fn test_corrupt_storage_falls_back_to_default() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::write(tempdir.path().join(STORAGE_FILE_PATH), b"invalid json").unwrap();

        let mut storage = AtomicStorage::new(open_tempdir(&tempdir)).await;
        assert_eq!(storage.get_string("channel").await, None);

        storage.set_string("channel", "stable").await.unwrap();
        storage.commit().await.unwrap();

        let storage = AtomicStorage::new(open_tempdir(&tempdir)).await;
        assert_eq!(storage.get_string("channel").await, Some("stable".to_string()));
    }

    #[fuchsia::test]
    async fn test_commit_error_on_read_only_dir() {
        let tempdir = tempfile::tempdir().unwrap();
        let read_only_dir = fuchsia_fs::directory::open_in_namespace(
            tempdir.path().to_str().unwrap(),
            fio::PERM_READABLE,
        )
        .unwrap();
        let mut storage = AtomicStorage::new(read_only_dir).await;
        storage.set_string("channel", "stable").await.unwrap();
        std::assert_matches!(storage.commit().await, Err(StorageError::OpenTempFile(_)));
    }
}
