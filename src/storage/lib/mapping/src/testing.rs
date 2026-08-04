// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use delivery_blob::DataBuffer;
use delivery_blob::compression::ChunkedArchiveError;
use fuchsia_sync::Mutex;
use std::sync::Arc;
use storage_ptr_slice::MutPtrByteSlice;

#[derive(Default)]
pub struct TestVecBufferInner {
    pub commits: Vec<(u64, usize)>,
    pub output: Vec<u8>,
}

#[derive(Clone)]
pub struct TestVecBufferReceiver(pub Arc<Mutex<TestVecBufferInner>>);

impl TestVecBufferReceiver {
    pub fn commits(&self) -> Vec<(u64, usize)> {
        self.0.lock().commits.clone()
    }

    pub fn output(&self) -> Vec<u8> {
        self.0.lock().output.clone()
    }
}

pub struct TestVecBuffer {
    pub data: Vec<u8>,
    pub committed_len: usize,
    pub offset: u64,
    pub receiver: TestVecBufferReceiver,
}

impl TestVecBuffer {
    pub fn new(size: usize) -> (Self, TestVecBufferReceiver) {
        Self::new_with_offset(size, 0)
    }

    pub fn new_with_offset(size: usize, offset: u64) -> (Self, TestVecBufferReceiver) {
        let receiver = TestVecBufferReceiver(Arc::new(Mutex::new(TestVecBufferInner::default())));
        let buf =
            Self { data: vec![0u8; size], committed_len: 0, offset, receiver: receiver.clone() };
        (buf, receiver)
    }
}

impl Drop for TestVecBuffer {
    fn drop(&mut self) {
        self.receiver.0.lock().output = std::mem::take(&mut self.data);
    }
}

impl DataBuffer for TestVecBuffer {
    fn mut_ptr_slice(&mut self) -> MutPtrByteSlice<'_> {
        let remaining = &mut self.data[self.committed_len..];
        MutPtrByteSlice::from(remaining)
    }

    fn commit(&mut self, size: usize) -> Result<(), ChunkedArchiveError> {
        self.receiver.0.lock().commits.push((self.offset, size));
        self.offset += size as u64;
        self.committed_len += size;
        Ok(())
    }
}
