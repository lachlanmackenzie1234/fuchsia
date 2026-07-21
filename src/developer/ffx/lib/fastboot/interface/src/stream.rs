// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.
use crate::util::convert_log_err;
use crc32fast;
use std::cmp::min;
use std::range::Range;
use zerocopy::IntoBytes;

// Streaming flash is an optional protocol used to reduce bandwidth,
// break up images that are too large for the device download buffer,
// and leverage device async I/O to speed partition flashing.
//
// Terminology used for the data structures in this module:
// * A `StreamCommandList` is a partition image and a list of commands
//   to write it to the target partition.
// * A `StreamCommand` is a range within the target partition and a description
//   of what the device should do within that range to generate data to write.
// * A `StreamOp` is a description of the bytes defining the owning `StreamCommand`:
//   either a single repeated 32 bit integer, or arbitrary data with a CRC32 checksum.
//
// `Chunk` and `Payload` correspond to `StreamCommand` and `StreamOp` but are
// used as intermediate structures while processing the partition image into operations.

#[derive(Debug, PartialEq, Eq)]
pub enum StreamOp {
    /// Arbitrary data backed by a checksum
    Flash { crc32: u32 },
    /// A repeated value
    Fill { val: u32 },
}

impl StreamOp {
    fn from_data(data: &[u32], range: Range<u64>) -> Self {
        Self::Flash { crc32: crc32fast::hash(&(data[convert_range(range)]).as_bytes()) }
    }

    fn from_fill(val: u32) -> Self {
        Self::Fill { val }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub struct StreamCommand {
    /// The range within the partition to write.
    pub range: Range<u64>,
    /// The operation to generate write data.
    pub op: StreamOp,
}

impl<'a> StreamCommand {
    fn from_data(data: &[u32], range: Range<u64>) -> Self {
        Self { range, op: StreamOp::from_data(data, range) }
    }

    fn from_fill(val: u32, range: Range<u64>) -> Self {
        Self { range, op: StreamOp::from_fill(val) }
    }
}

pub struct StreamCommandList<'a> {
    data: &'a [u32],
    commands: Vec<StreamCommand>,
}

impl<'a> StreamCommandList<'a> {
    /// Iterate over the commands and associated data segments.
    pub fn commands_iter(&self) -> impl Iterator<Item = (&StreamCommand, &'a [u32])> {
        self.commands.iter().map(|c| (c, &self.data[convert_range(c.range)]))
    }
}

struct Chunk {
    range: Range<u64>,
    payload: Payload,
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum Payload {
    Fill(u32),
    Flash,
}

impl Payload {
    fn from_segment(segment: &[u32]) -> Self {
        // If segment.iter().next().is_none(),
        // we get a 0-length Fill segment with a payload of 0,
        // which is a no-op.
        // Technically we could filter these no-op commands,
        // but even though they are syntactically possible in this context
        // they aren't possible due to the way that chunk_range is constructed.
        let val = segment.iter().next().unwrap_or(&0);
        if segment.iter().all(|v| v == val) { Self::Fill(*val) } else { Self::Flash }
    }
}

fn range_length<T: std::ops::Sub>(range: Range<T>) -> <T as std::ops::Sub>::Output {
    let Range { start, end } = range;
    end - start
}

impl Chunk {
    fn from_payload(range: Range<u64>, payload: Payload) -> Self {
        Self { range, payload }
    }

    fn from_data(range: Range<u64>, data: &[u32]) -> Self {
        Self::from_payload(range, Payload::from_segment(&data[convert_range(range)]))
    }

    fn len_words(&self) -> u64 {
        range_length(self.range)
    }

    fn with_extension(self, length: u64) -> Self {
        let Self { range: Range { start, end }, payload } = self;
        Self { range: Range { start, end: end + length }, payload }
    }
}

fn convert_range<T, U>(r: Range<T>) -> Range<U>
where
    U: TryFrom<T>,
    <U as TryFrom<T>>::Error: std::error::Error,
{
    Range { start: convert_log_err(r.start).unwrap(), end: convert_log_err(r.end).unwrap() }
}

// Process the segment into the work-in-progress chunk.
// The current chunk is either expanded to include the new segment OR
// is wrapped up into a completed command, pushed to the command vector,
// and a new work-in-progress chunk is returned.
fn process_segment(
    data: &[u32],
    segment_range: Range<u64>,
    max_download_words: u64,
    commands: &mut Vec<StreamCommand>,
    chunk: Chunk,
) -> Chunk {
    use Payload::*;

    let segment_len_words = range_length(segment_range);
    let new_payload = Payload::from_segment(&data[convert_range(segment_range)]);
    match (chunk.payload, new_payload) {
        // New payload is a flash or a fill with a different value
        (p1 @ Fill(val), p2) if p1 != p2 => {
            commands.push(StreamCommand::from_fill(val, chunk.range));
            Chunk::from_payload(segment_range, new_payload)
        }
        (Flash, Fill(_)) => {
            commands.push(StreamCommand::from_data(data, chunk.range));
            Chunk::from_payload(segment_range, new_payload)
        }
        (Flash, Flash)
            if chunk.len_words().checked_add(segment_len_words).expect("Integer overflow")
                > max_download_words =>
        {
            // Can't have a Flash segment longer than max_download_words.
            // Even though the following chunk is also raw data, we need to
            // split it up into multiple commands.
            commands.push(StreamCommand::from_data(data, chunk.range));
            Chunk::from_payload(segment_range, new_payload)
        }
        // Matching fill or under max size flash
        _ => chunk.with_extension(segment_len_words),
    }
}

/// Generate a list of streaming flash commands.
/// Takes a partition image, the max download size the device supports,
/// the streaming segment size, and the partition offset from the start of the device.
///
/// Note: `data` is a slice of u32 and `max_download_words`, `segment_size_words`,
/// and `partition_start_word` are all denominated in u32 to enforce good alignment
/// for checking if a segment can be represented as Fill, which takes a u32 value.
pub fn generate_command_list<'a>(
    data: &'a [u32],
    max_download_words: u64,
    segment_size_words: u64,
    partition_start_word: u64,
) -> StreamCommandList<'a> {
    let prefix_words =
        partition_start_word.next_multiple_of(segment_size_words) - partition_start_word;

    let data_len: u64 = convert_log_err(data.len()).unwrap();
    let prefix_words = if prefix_words == 0 { segment_size_words } else { prefix_words };
    // The first chunk primes the process_segment sequence
    // and handles a possible unaligned prefix.
    let mut chunk = Chunk::from_data(Range { start: 0, end: min(prefix_words, data_len) }, data);

    let mut commands = vec![];
    for segment_range in (prefix_words..data_len)
        .step_by(convert_log_err(segment_size_words).unwrap())
        .map(|start| Range { start, end: start + min(segment_size_words, data_len - start) })
    {
        chunk = process_segment(data, segment_range, max_download_words, &mut commands, chunk);
    }

    let operation = match chunk.payload {
        Payload::Fill(val) => StreamCommand::from_fill(val, chunk.range),
        Payload::Flash => StreamCommand::from_data(data, chunk.range),
    };
    commands.push(operation);

    StreamCommandList { data, commands }
}

#[cfg(test)]
mod test {
    use super::*;

    impl StreamCommand {
        fn new(start: u64, end: u64, op: StreamOp) -> Self {
            Self { range: Range { start, end }, op }
        }
    }

    macro_rules! multi_chain (
        ($datum:expr $(,)?) => {
            $datum
        };
        ($datum_1:expr, $datum_2:expr $(,)?) => {
            multi_chain!(core::iter::chain($datum_1, $datum_2))
        };
        ($datum_1:expr, $( $data:expr ),+ $(,)? ) => {
            multi_chain!(core::iter::chain($datum_1, multi_chain!($($data,)*)))
        };
    );

    #[fuchsia::test()]
    fn test_stream_command_generation_basic() {
        const SEGMENT_SIZE_WORDS: u32 = 1024;
        let flash_data = 0..SEGMENT_SIZE_WORDS;
        let fill_data = std::iter::repeat(0u32).take(SEGMENT_SIZE_WORDS as usize);
        let data = multi_chain!(
            flash_data.clone(),
            fill_data.clone(),
            fill_data.clone(),
            fill_data.clone(),
            flash_data.clone(),
        )
        .collect::<Vec<_>>();

        let command_list =
            generate_command_list(data.as_slice(), 1 << 28, SEGMENT_SIZE_WORDS.into(), 0);

        let crc32 = 0xf15f689b;
        assert_eq!(
            command_list.commands,
            [
                StreamCommand::new(0, 1024, StreamOp::Flash { crc32 }),
                StreamCommand::new(1024, 4096, StreamOp::Fill { val: 0 }),
                StreamCommand::new(4096, 5120, StreamOp::Flash { crc32 }),
            ]
        );
    }

    #[fuchsia::test()]
    fn test_stream_command_generation_different_fill() {
        const SEGMENT_SIZE_WORDS: u32 = 1024;
        let fill_zero = std::iter::repeat(0u32).take(SEGMENT_SIZE_WORDS as usize);
        let fill_one = std::iter::repeat(1u32).take(SEGMENT_SIZE_WORDS as usize);

        let data = multi_chain!(
            fill_zero.clone(),
            fill_zero.clone(),
            fill_one.clone(),
            fill_one.clone(),
            fill_zero.clone(),
            fill_one.clone(),
            fill_one.clone(),
            fill_one.clone(),
            fill_one.clone(),
            fill_zero.clone(),
            fill_zero.clone(),
        )
        .collect::<Vec<_>>();

        let command_list =
            generate_command_list(data.as_slice(), 1 << 28, SEGMENT_SIZE_WORDS.into(), 0);

        assert_eq!(
            command_list.commands,
            [
                StreamCommand::new(0, 2048, StreamOp::Fill { val: 0 }),
                StreamCommand::new(2048, 4096, StreamOp::Fill { val: 1 }),
                StreamCommand::new(4096, 5120, StreamOp::Fill { val: 0 }),
                StreamCommand::new(5120, 9216, StreamOp::Fill { val: 1 }),
                StreamCommand::new(9216, 11264, StreamOp::Fill { val: 0 })
            ]
        );
    }

    #[fuchsia::test()]
    fn test_stream_command_generation_split_flash() {
        const SEGMENT_SIZE_WORDS: u32 = 1024;

        let flash_data = 0..SEGMENT_SIZE_WORDS;
        let data = multi_chain!(
            flash_data.clone(),
            flash_data.clone(),
            flash_data.clone(),
            flash_data.clone(),
            flash_data.clone(),
            flash_data.clone(),
            flash_data.clone(),
            flash_data.clone(),
        )
        .collect::<Vec<_>>();

        let command_list = generate_command_list(
            data.as_slice(),
            (SEGMENT_SIZE_WORDS * 2).into(),
            SEGMENT_SIZE_WORDS.into(),
            0,
        );

        let crc32 = crc32fast::hash(
            multi_chain!(flash_data.clone(), flash_data.clone())
                .collect::<Vec<_>>()
                .as_slice()
                .as_bytes(),
        );
        assert_eq!(
            command_list.commands,
            [
                StreamCommand::new(0, 2048, StreamOp::Flash { crc32 }),
                StreamCommand::new(2048, 4096, StreamOp::Flash { crc32 }),
                StreamCommand::new(4096, 6144, StreamOp::Flash { crc32 }),
                StreamCommand::new(6144, 8192, StreamOp::Flash { crc32 }),
            ]
        );
    }

    #[fuchsia::test()]
    fn test_stream_command_generation_unaligned_prefix() {
        const SEGMENT_SIZE_WORDS: u32 = 1024;

        let flash_data = 0..SEGMENT_SIZE_WORDS;
        let data = multi_chain!(
            0..(SEGMENT_SIZE_WORDS / 2),
            flash_data.clone(),
            flash_data.clone(),
            flash_data.clone(),
            flash_data.clone()
        )
        .collect::<Vec<_>>();

        let command_list = generate_command_list(
            data.as_slice(),
            1 << 28,
            SEGMENT_SIZE_WORDS.into(),
            (SEGMENT_SIZE_WORDS / 2).into(),
        );

        assert_eq!(
            command_list.commands,
            [StreamCommand::new(0, 4608, StreamOp::Flash { crc32: 0x7fb83cdc })]
        );
    }

    #[fuchsia::test()]
    fn test_stream_command_generation_trailing_suffix() {
        const SEGMENT_SIZE_WORDS: u32 = 1024;

        let fill_zero = std::iter::repeat(0u32).take(SEGMENT_SIZE_WORDS as usize);
        let data = multi_chain!(
            fill_zero.clone(),
            fill_zero.clone(),
            fill_zero.clone(),
            fill_zero.clone(),
            0..(SEGMENT_SIZE_WORDS / 2)
        )
        .collect::<Vec<_>>();

        let command_list =
            generate_command_list(data.as_slice(), 1 << 28, SEGMENT_SIZE_WORDS.into(), 0);

        assert_eq!(
            command_list.commands,
            [
                StreamCommand::new(0, 4096, StreamOp::Fill { val: 0 }),
                StreamCommand::new(4096, 4608, StreamOp::Flash { crc32: 0x6feca6e2 })
            ]
        );
    }

    #[fuchsia::test()]
    fn test_stream_command_generation_tiny_image() {
        const SEGMENT_SIZE_WORDS: u32 = 1024;

        let data = vec![0u32; 8];
        let command_list =
            generate_command_list(data.as_slice(), 1 << 28, SEGMENT_SIZE_WORDS.into(), 0);

        assert_eq!(command_list.commands, [StreamCommand::new(0, 8, StreamOp::Fill { val: 0 })]);
    }
}
