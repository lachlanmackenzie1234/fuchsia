// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::compression::{ChunkedArchiveError, CompressionAlgorithm, ThreadLocalDecompressor};
use std::ops::Range;

#[derive(Clone)]
pub struct CompressionInfo {
    chunk_size: u64,
    compressed_size: u64,
    // The chunked compression format stores 0 as the first offset but it's not stored here. Not
    // storing the 0 avoids the allocation for blobs smaller than the chunk size.
    small_offsets: Box<[u32]>,
    large_offsets: Box<[u64]>,
    decompressor: ThreadLocalDecompressor,
}

impl CompressionInfo {
    pub fn new(
        chunk_size: u64,
        compressed_size: u64,
        offsets: &[u64],
        compression_algorithm: CompressionAlgorithm,
    ) -> Result<Self, ChunkedArchiveError> {
        let decompressor = compression_algorithm.thread_local_decompressor();
        if chunk_size == 0 {
            return Err(ChunkedArchiveError::IntegrityError);
        } else if offsets.is_empty() || *offsets.first().unwrap() != 0 {
            // There should always be at least 1 offset and the first offset must always be 0.
            return Err(ChunkedArchiveError::IntegrityError);
        } else if !offsets.array_windows().all(|[a, b]| a < b) {
            // The offsets must be in ascending order.
            return Err(ChunkedArchiveError::IntegrityError);
        } else if offsets.len() == 1 {
            // Simple case where the blob is smaller than the chunk size so only the 0 offset is
            // present. The 0 isn't stored so no allocation is necessary.
            Ok(Self {
                chunk_size,
                compressed_size,
                small_offsets: Box::default(),
                large_offsets: Box::default(),
                decompressor,
            })
        } else if *offsets.last().unwrap() <= u32::MAX as u64 {
            // Check the last index first since most compressed blobs are going to be smaller
            // than 4GiB making all offsets small.
            Ok(Self {
                chunk_size,
                compressed_size,
                small_offsets: offsets[1..].iter().map(|x| *x as u32).collect(),
                large_offsets: Box::default(),
                decompressor,
            })
        } else {
            // The partition point is the index of the first compressed offset that's > u32::MAX.
            let partition_point = offsets.partition_point(|&x| x <= u32::MAX as u64);
            Ok(Self {
                chunk_size,
                compressed_size,
                small_offsets: offsets[1..partition_point].iter().map(|x| *x as u32).collect(),
                large_offsets: offsets[partition_point..].into(),
                decompressor,
            })
        }
    }

    /// Returns the chunk size for this compressed blob.
    pub fn chunk_size(&self) -> u64 {
        self.chunk_size
    }

    /// Returns the total compressed size of this blob.
    pub fn compressed_size(&self) -> u64 {
        self.compressed_size
    }

    /// Returns the compressed range for the specified uncompressed range.
    pub fn compressed_range_for_uncompressed_range(
        &self,
        range: &Range<u64>,
    ) -> Result<Range<u64>, ChunkedArchiveError> {
        if range.start % self.chunk_size != 0 || range.start >= range.end {
            return Err(ChunkedArchiveError::IntegrityError);
        }

        let start_chunk_index = (range.start / self.chunk_size) as usize;
        let start_offset = self
            .compressed_offset_for_chunk_index(start_chunk_index)
            .ok_or(ChunkedArchiveError::OutOfRange)?;

        // The end of the range may not be aligned to the chunk size for the last chunk.
        let end_chunk_index = range.end.div_ceil(self.chunk_size) as usize;
        let end_offset = match self.compressed_offset_for_chunk_index(end_chunk_index) {
            None => self.compressed_size,
            Some(offset) => {
                // This isn't the last chunk so the end must be aligned.
                if !range.end.is_multiple_of(self.chunk_size) {
                    return Err(ChunkedArchiveError::IntegrityError);
                }
                // `CompressionInfo::new` validates that all of the offsets are ascending.
                offset
            }
        };

        Ok(start_offset..end_offset)
    }

    fn compressed_offset_for_chunk_index(&self, chunk_index: usize) -> Option<u64> {
        if chunk_index == 0 {
            Some(0)
        } else if chunk_index - 1 < self.small_offsets.len() {
            Some(self.small_offsets[chunk_index - 1] as u64)
        } else if chunk_index - 1 - self.small_offsets.len() < self.large_offsets.len() {
            Some(self.large_offsets[chunk_index - 1 - self.small_offsets.len()])
        } else {
            None
        }
    }

    /// Decompress the bytes of `src` into `dst`.
    ///   - `src` is allowed to span multiple chunks.
    ///   - `dst` must have the exact size of the uncompressed bytes.
    ///   - `dst_start_offset` is the location of the uncompressed bytes within the blob and must be
    ///     chunk aligned. This is necessary for determining the chunk boundaries in `src`.
    pub fn decompress(
        &self,
        mut src: &[u8],
        mut dst: &mut [u8],
        dst_start_offset: u64,
    ) -> Result<(), ChunkedArchiveError> {
        if dst_start_offset % self.chunk_size != 0 {
            return Err(ChunkedArchiveError::IntegrityError);
        }

        let start_chunk_index = (dst_start_offset / self.chunk_size) as usize;
        let chunk_count = dst.len().div_ceil(self.chunk_size as usize);
        let mut start_offset = self
            .compressed_offset_for_chunk_index(start_chunk_index)
            .ok_or(ChunkedArchiveError::IntegrityError)?;

        // Decompress each chunk individually.
        for chunk_index in start_chunk_index..(start_chunk_index + chunk_count) {
            match self.compressed_offset_for_chunk_index(chunk_index + 1) {
                Some(end_offset) => {
                    let (to_decompress, src_remaining) = src
                        .split_at_checked((end_offset - start_offset) as usize)
                        .ok_or(ChunkedArchiveError::IntegrityError)?;
                    let (to_decompress_into, dst_remaining) = dst
                        .split_at_mut_checked(self.chunk_size as usize)
                        .ok_or(ChunkedArchiveError::IntegrityError)?;

                    let decompressed_bytes = self.decompressor.decompress_into(
                        to_decompress,
                        to_decompress_into,
                        chunk_index,
                    )?;
                    if decompressed_bytes != to_decompress_into.len() {
                        return Err(ChunkedArchiveError::IntegrityError);
                    }
                    src = src_remaining;
                    dst = dst_remaining;
                    start_offset = end_offset;
                }
                None => {
                    let decompressed_bytes =
                        self.decompressor.decompress_into(src, dst, chunk_index)?;
                    if decompressed_bytes != dst.len() {
                        return Err(ChunkedArchiveError::IntegrityError);
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compression_info_new_small_and_large_offsets() {
        let info = CompressionInfo::new(4096, 50, &[0], CompressionAlgorithm::Zstd).unwrap();
        assert_eq!(info.chunk_size(), 4096);
        assert_eq!(info.compressed_size(), 50);
        assert_eq!(info.compressed_offset_for_chunk_index(0), Some(0));
        assert_eq!(info.compressed_offset_for_chunk_index(1), None);

        let info =
            CompressionInfo::new(4096, 350, &[0, 100, 250], CompressionAlgorithm::Zstd).unwrap();
        assert_eq!(info.compressed_offset_for_chunk_index(0), Some(0));
        assert_eq!(info.compressed_offset_for_chunk_index(1), Some(100));
        assert_eq!(info.compressed_offset_for_chunk_index(2), Some(250));
        assert_eq!(info.compressed_offset_for_chunk_index(3), None);

        let large_val = u32::MAX as u64 + 1000;
        let info = CompressionInfo::new(
            4096,
            large_val + 500,
            &[0, 500, large_val],
            CompressionAlgorithm::Zstd,
        )
        .unwrap();
        assert_eq!(info.compressed_offset_for_chunk_index(0), Some(0));
        assert_eq!(info.compressed_offset_for_chunk_index(1), Some(500));
        assert_eq!(info.compressed_offset_for_chunk_index(2), Some(large_val));
        assert_eq!(info.compressed_offset_for_chunk_index(3), None);
    }

    #[test]
    fn test_compressed_range_for_uncompressed_range() {
        let info = CompressionInfo::new(4096, 500, &[0, 100, 250, 400], CompressionAlgorithm::Zstd)
            .unwrap();
        let range = info.compressed_range_for_uncompressed_range(&(0..4096)).unwrap();
        assert_eq!(range, 0..100);

        let range = info.compressed_range_for_uncompressed_range(&(4096..12288)).unwrap();
        assert_eq!(range, 100..400);

        let range = info.compressed_range_for_uncompressed_range(&(4096..16384)).unwrap();
        assert_eq!(range, 100..500);
    }

    #[test]
    fn test_compression_info_offsets_must_start_with_zero() {
        assert!(CompressionInfo::new(4096, 100, &[], CompressionAlgorithm::Zstd).is_err());
        assert!(CompressionInfo::new(4096, 100, &[1], CompressionAlgorithm::Zstd).is_err());
        assert!(CompressionInfo::new(4096, 100, &[0], CompressionAlgorithm::Zstd).is_ok());
    }

    #[test]
    fn test_compression_info_offsets_must_be_sorted() {
        assert!(CompressionInfo::new(4096, 100, &[0, 1, 2], CompressionAlgorithm::Zstd).is_ok());
        assert!(CompressionInfo::new(4096, 100, &[0, 2, 1], CompressionAlgorithm::Zstd).is_err());
        assert!(CompressionInfo::new(4096, 100, &[0, 1, 1], CompressionAlgorithm::Zstd).is_err());
    }

    #[test]
    fn test_compression_info_splitting_offsets() {
        const MAX_SMALL_OFFSET: u64 = u32::MAX as u64;
        let compression_info =
            CompressionInfo::new(4096, 100, &[0], CompressionAlgorithm::Zstd).unwrap();
        assert!(compression_info.small_offsets.is_empty());
        assert!(compression_info.large_offsets.is_empty());

        let compression_info =
            CompressionInfo::new(4096, 20, &[0, 10], CompressionAlgorithm::Zstd).unwrap();
        assert_eq!(&*compression_info.small_offsets, &[10]);
        assert!(compression_info.large_offsets.is_empty());

        let compression_info =
            CompressionInfo::new(4096, 40, &[0, 10, 20, 30], CompressionAlgorithm::Zstd).unwrap();
        assert_eq!(&*compression_info.small_offsets, &[10, 20, 30]);
        assert!(compression_info.large_offsets.is_empty());

        let compression_info = CompressionInfo::new(
            4096,
            MAX_SMALL_OFFSET,
            &[0, MAX_SMALL_OFFSET - 1],
            CompressionAlgorithm::Zstd,
        )
        .unwrap();
        assert_eq!(&*compression_info.small_offsets, &[u32::MAX - 1]);
        assert!(compression_info.large_offsets.is_empty());

        let compression_info = CompressionInfo::new(
            4096,
            MAX_SMALL_OFFSET + 1,
            &[0, MAX_SMALL_OFFSET],
            CompressionAlgorithm::Zstd,
        )
        .unwrap();
        assert_eq!(&*compression_info.small_offsets, &[u32::MAX]);
        assert!(compression_info.large_offsets.is_empty());

        let compression_info = CompressionInfo::new(
            4096,
            MAX_SMALL_OFFSET + 2,
            &[0, MAX_SMALL_OFFSET + 1],
            CompressionAlgorithm::Zstd,
        )
        .unwrap();
        assert!(compression_info.small_offsets.is_empty());
        assert_eq!(&*compression_info.large_offsets, &[MAX_SMALL_OFFSET + 1]);

        let compression_info = CompressionInfo::new(
            4096,
            MAX_SMALL_OFFSET + 2,
            &[0, MAX_SMALL_OFFSET - 1, MAX_SMALL_OFFSET, MAX_SMALL_OFFSET + 1],
            CompressionAlgorithm::Zstd,
        )
        .unwrap();
        assert_eq!(&*compression_info.small_offsets, &[u32::MAX - 1, u32::MAX]);
        assert_eq!(&*compression_info.large_offsets, &[MAX_SMALL_OFFSET + 1]);

        let compression_info = CompressionInfo::new(
            4096,
            MAX_SMALL_OFFSET + 20,
            &[0, MAX_SMALL_OFFSET + 10],
            CompressionAlgorithm::Zstd,
        )
        .unwrap();
        assert!(compression_info.small_offsets.is_empty());
        assert_eq!(&*compression_info.large_offsets, &[MAX_SMALL_OFFSET + 10]);

        let compression_info = CompressionInfo::new(
            4096,
            MAX_SMALL_OFFSET + 30,
            &[0, MAX_SMALL_OFFSET + 10, MAX_SMALL_OFFSET + 20],
            CompressionAlgorithm::Zstd,
        )
        .unwrap();
        assert!(compression_info.small_offsets.is_empty());
        assert_eq!(
            &*compression_info.large_offsets,
            &[MAX_SMALL_OFFSET + 10, MAX_SMALL_OFFSET + 20]
        );
    }
}
