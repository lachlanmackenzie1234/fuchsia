// Copyright 2021 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use anyhow::{Context, Result, format_err};
use display_utils::{Image, ImageId};
use fuchsia_image_format::{
    image_format_minimum_row_bytes_2, image_format_stride_bytes_per_width_pixel_2,
};

use mapped_vmo::Mapping;
use std::cell::Cell;
use std::cmp::min;

// TODO(armansito): Extend this to support different patterns and tiled image formats.

pub struct MappedImage {
    image: Image,
    surface: Surface,

    /// Time spent in [`MappedImage::cache_clean`] since the last
    /// [`MappedImage::take_clean_time`] call.
    clean_time: Cell<zx::MonotonicDuration>,
}

pub struct Frame {
    pub pos_x: u32,
    pub pos_y: u32,
    pub width: u32,
    pub height: u32,
}

/// The CPU-writable pixels of a linear image, mapped into memory.
///
/// Holds no display resources, so drawing can be tested over any VMO.
pub struct Surface {
    mapping: Mapping,
    width: u32,
    height: u32,
    pixel_width: u32,
    row_bytes: u32,
}

impl Surface {
    /// `pixel_width` is the size of one pixel in bytes, and `row_bytes` the distance between the
    /// starts of two consecutive rows.
    pub fn new(
        mapping: Mapping,
        width: u32,
        height: u32,
        pixel_width: u32,
        row_bytes: u32,
    ) -> Self {
        Surface { mapping, width, height, pixel_width, row_bytes }
    }

    /// Fill the specified region of this surface with the specified color. The size and
    /// interpretation of the given color must match the underlying image format.
    pub fn fill_region(&self, color: &[u8], frame: &Frame) -> Result<()> {
        if self.pixel_width != u32::try_from(color.len())? {
            return Err(format_err!(
                "provided color width ({}) does not match expected pixel format width ({})",
                color.len(),
                self.pixel_width
            ));
        }

        // First clip the frame to the image bounds.
        let frame = {
            let pos_x = min(self.width - 1, frame.pos_x);
            let pos_y = min(self.height - 1, frame.pos_y);
            Frame {
                pos_x,
                pos_y,
                width: min(frame.width, self.width - pos_x),
                height: min(frame.height, self.height - pos_y),
            }
        };

        // Color the pixels within the frame.
        let pixel_stride = self.row_bytes / self.width;
        if pixel_stride == self.pixel_width {
            // The pixels of each row of the frame are contiguous, so each row is written with a
            // single copy. This is much faster than one `write_at()` call per pixel.
            let row = color.repeat(usize::try_from(frame.width)?);
            let row_bytes = usize::try_from(self.row_bytes)?;
            let first_row_offset = row_bytes * usize::try_from(frame.pos_y)?
                + usize::try_from(pixel_stride)? * usize::try_from(frame.pos_x)?;
            for row_index in 0..usize::try_from(frame.height)? {
                self.mapping.write_at(first_row_offset + row_bytes * row_index, &row);
            }
        } else {
            for row in 0..frame.height {
                for col in 0..frame.width {
                    let idx =
                        self.row_bytes * (row + frame.pos_y) + pixel_stride * (col + frame.pos_x);
                    self.mapping.write_at(idx.try_into()?, color);
                }
            }
        }

        Ok(())
    }
}

impl MappedImage {
    pub fn create(image: Image) -> Result<MappedImage> {
        let size: usize = *image.buffer_settings.size_bytes.as_ref().unwrap() as usize;
        let mapping = Mapping::create_from_vmo(
            &image.vmo,
            size,
            zx::VmarFlags::PERM_READ | zx::VmarFlags::PERM_WRITE,
        )
        .context("failed to map VMO")?;

        let constraints = &image.format_constraints;
        let pixel_width = image_format_stride_bytes_per_width_pixel_2(
            *constraints.pixel_format.as_ref().unwrap(),
        )
        .unwrap();
        let row_bytes = image_format_minimum_row_bytes_2(constraints, image.parameters.width)?;
        let surface = Surface::new(
            mapping,
            image.parameters.width,
            image.parameters.height,
            pixel_width,
            row_bytes,
        );
        Ok(MappedImage { image, surface, clean_time: Cell::new(zx::MonotonicDuration::ZERO) })
    }

    pub fn id(&self) -> ImageId {
        self.image.id
    }

    pub fn zero(&self) -> Result<()> {
        self.image.vmo.op_range(zx::VmoOp::ZERO, 0, self.image.vmo.get_size()?)?;
        Ok(())
    }

    /// Fill the image with the specified color. The size and interpretation of the given color
    /// must match the underlying image format.
    pub fn fill(&self, color: &[u8]) -> Result<()> {
        self.fill_region(
            color,
            &Frame { pos_x: 0, pos_y: 0, width: self.width(), height: self.height() },
        )
    }

    /// Fill the specified region of this image with the specified color. The size and
    /// interpretation of the given color must match the underlying image format.
    pub fn fill_region(&self, color: &[u8], frame: &Frame) -> Result<()> {
        self.surface.fill_region(color, frame)
    }

    /// Clean (write back) data caches, so previous writes are visible in main memory. This
    /// operation must be called at once before this image buffer gets presented to avoid artifacts
    /// during scanout in height refresh rates if the image is changing frequently. This operation
    /// can be expensive on large images and should be performed sparingly.
    pub fn cache_clean(&self) -> Result<()> {
        let start = zx::MonotonicInstant::get();
        self.image.vmo.op_range(zx::VmoOp::CACHE_CLEAN, 0, self.image.vmo.get_size()?)?;
        self.clean_time.set(self.clean_time.get() + (zx::MonotonicInstant::get() - start));
        Ok(())
    }

    /// Returns the time spent in [`MappedImage::cache_clean`] since the previous call, and resets
    /// the count.
    pub fn take_clean_time(&self) -> zx::MonotonicDuration {
        self.clean_time.replace(zx::MonotonicDuration::ZERO)
    }

    fn width(&self) -> u32 {
        self.image.parameters.width
    }

    fn height(&self) -> u32 {
        self.image.parameters.height
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use googletest::{expect_eq, expect_true, gtest};

    /// The per-pixel algorithm `fill_region` used before rows were copied at once, over a plain
    /// buffer. Results must match it byte for byte.
    fn reference_fill_region(
        buffer: &mut [u8],
        width: u32,
        height: u32,
        row_bytes: u32,
        color: &[u8],
        frame: &Frame,
    ) {
        let pos_x = min(width - 1, frame.pos_x);
        let pos_y = min(height - 1, frame.pos_y);
        let frame_width = min(frame.width, width - pos_x);
        let frame_height = min(frame.height, height - pos_y);
        let pixel_stride = row_bytes / width;
        for row in 0..frame_height {
            for col in 0..frame_width {
                let idx = (row_bytes * (row + pos_y) + pixel_stride * (col + pos_x)) as usize;
                buffer[idx..idx + color.len()].copy_from_slice(color);
            }
        }
    }

    /// A [`Surface`] over a test VMO, plus the bytes it is expected to hold.
    struct Fixture {
        surface: Surface,
        expected: Vec<u8>,
    }

    impl Fixture {
        /// Creates a surface whose bytes start out as a non-zero pattern, so writes to the wrong
        /// place and missing writes both show up as differences.
        fn new(width: u32, height: u32, pixel_width: u32, row_bytes: u32) -> Fixture {
            let page_size = zx::system_get_page_size() as usize;
            let size = ((row_bytes * height) as usize).next_multiple_of(page_size);
            let (mapping, _vmo) = Mapping::allocate(size).expect("allocate test VMO");
            let expected: Vec<u8> = (0..size).map(|i| (i * 7 + 3) as u8).collect();
            mapping.write_at(0, &expected);
            Fixture {
                surface: Surface::new(mapping, width, height, pixel_width, row_bytes),
                expected,
            }
        }

        /// Fills `frame` in the surface and in the expected bytes.
        fn fill_region(&mut self, color: &[u8], frame: Frame) {
            self.surface.fill_region(color, &frame).expect("fill_region");
            let surface = &self.surface;
            reference_fill_region(
                &mut self.expected,
                surface.width,
                surface.height,
                surface.row_bytes,
                color,
                &frame,
            );
        }

        fn actual(&self) -> Vec<u8> {
            let mut actual = vec![0; self.surface.mapping.len()];
            self.surface.mapping.read_at(0, &mut actual);
            actual
        }

        /// Returns the offset of the first byte that differs from the reference, if any.
        fn first_difference(&self) -> Option<usize> {
            self.actual().iter().zip(self.expected.iter()).position(|(a, e)| a != e)
        }
    }

    const COLOR: [u8; 4] = [0x10, 0x20, 0x30, 0xff];

    #[gtest]
    #[fuchsia::test]
    fn reference_fills_exactly_the_frame() {
        // Checks the reference itself, by hand, on a 4x3 surface with 2 bytes of padding per row.
        let mut buffer = vec![0u8; 18 * 3];
        reference_fill_region(
            &mut buffer,
            4,
            3,
            18,
            &COLOR,
            &Frame { pos_x: 1, pos_y: 1, width: 2, height: 1 },
        );
        let mut expected = vec![0u8; 18 * 3];
        expected[22..26].copy_from_slice(&COLOR);
        expected[26..30].copy_from_slice(&COLOR);
        expect_eq!(buffer, expected);
    }

    #[gtest]
    #[fuchsia::test]
    fn fill_region_writes_only_the_frame() {
        let mut fixture = Fixture::new(4, 3, 4, 16);
        let before = fixture.actual();
        fixture.fill_region(&COLOR, Frame { pos_x: 1, pos_y: 1, width: 2, height: 2 });
        let actual = fixture.actual();
        for (offset, (byte, old)) in actual.iter().zip(before.iter()).enumerate() {
            let row = offset / 16;
            let col = (offset % 16) / 4;
            if (1..3).contains(&row) && (1..3).contains(&col) {
                expect_eq!(*byte, COLOR[offset % 4], "offset {}", offset);
            } else {
                expect_eq!(*byte, *old, "offset {}", offset);
            }
        }
    }

    #[gtest]
    #[fuchsia::test]
    fn fill_whole_surface() {
        let mut fixture = Fixture::new(64, 48, 4, 256);
        fixture.fill_region(&COLOR, Frame { pos_x: 0, pos_y: 0, width: 64, height: 48 });
        expect_eq!(fixture.first_difference(), None);
    }

    #[gtest]
    #[fuchsia::test]
    fn fill_interior_rect() {
        let mut fixture = Fixture::new(64, 48, 4, 256);
        fixture.fill_region(&COLOR, Frame { pos_x: 5, pos_y: 7, width: 20, height: 11 });
        expect_eq!(fixture.first_difference(), None);
    }

    #[gtest]
    #[fuchsia::test]
    fn fill_rect_clipped_at_right_edge() {
        let mut fixture = Fixture::new(64, 48, 4, 256);
        fixture.fill_region(&COLOR, Frame { pos_x: 50, pos_y: 3, width: 30, height: 10 });
        expect_eq!(fixture.first_difference(), None);
    }

    #[gtest]
    #[fuchsia::test]
    fn fill_rect_clipped_at_bottom_edge() {
        let mut fixture = Fixture::new(64, 48, 4, 256);
        fixture.fill_region(&COLOR, Frame { pos_x: 3, pos_y: 40, width: 10, height: 30 });
        expect_eq!(fixture.first_difference(), None);
    }

    #[gtest]
    #[fuchsia::test]
    fn fill_rect_clipped_at_bottom_right_corner() {
        let mut fixture = Fixture::new(64, 48, 4, 256);
        fixture
            .fill_region(&COLOR, Frame { pos_x: 60, pos_y: 45, width: u32::MAX, height: u32::MAX });
        expect_eq!(fixture.first_difference(), None);
    }

    #[gtest]
    #[fuchsia::test]
    fn fill_origin_past_edge_draws_last_column_and_row() {
        let mut fixture = Fixture::new(64, 48, 4, 256);
        fixture.fill_region(&COLOR, Frame { pos_x: 100, pos_y: 5, width: 10, height: 3 });
        fixture.fill_region(&[1, 2, 3, 4], Frame { pos_x: 5, pos_y: 100, width: 3, height: 10 });
        fixture.fill_region(&[5, 6, 7, 8], Frame { pos_x: 64, pos_y: 48, width: 2, height: 2 });
        expect_eq!(fixture.first_difference(), None);
    }

    #[gtest]
    #[fuchsia::test]
    fn fill_one_pixel() {
        let mut fixture = Fixture::new(64, 48, 4, 256);
        fixture.fill_region(&COLOR, Frame { pos_x: 17, pos_y: 9, width: 1, height: 1 });
        expect_eq!(fixture.first_difference(), None);
    }

    #[gtest]
    #[fuchsia::test]
    fn fill_one_pixel_surface() {
        let mut fixture = Fixture::new(1, 1, 4, 64);
        fixture.fill_region(&COLOR, Frame { pos_x: 0, pos_y: 0, width: 1, height: 1 });
        expect_eq!(fixture.first_difference(), None);
    }

    #[gtest]
    #[fuchsia::test]
    fn fill_empty_frame_writes_nothing() {
        let mut fixture = Fixture::new(64, 48, 4, 256);
        fixture.fill_region(&COLOR, Frame { pos_x: 5, pos_y: 5, width: 0, height: 10 });
        fixture.fill_region(&COLOR, Frame { pos_x: 5, pos_y: 5, width: 10, height: 0 });
        expect_eq!(fixture.first_difference(), None);
    }

    #[gtest]
    #[fuchsia::test]
    fn fill_three_byte_pixels() {
        // Rows without padding.
        let mut fixture = Fixture::new(10, 6, 3, 30);
        fixture.fill_region(&[1, 2, 3], Frame { pos_x: 2, pos_y: 1, width: 5, height: 3 });
        fixture.fill_region(&[4, 5, 6], Frame { pos_x: 8, pos_y: 4, width: 5, height: 5 });
        expect_eq!(fixture.first_difference(), None);

        // Rows padded by less than a pixel per pixel, so the pixel stride is still 3 bytes.
        let mut fixture = Fixture::new(10, 6, 3, 32);
        fixture.fill_region(&[1, 2, 3], Frame { pos_x: 2, pos_y: 1, width: 5, height: 3 });
        fixture.fill_region(&[4, 5, 6], Frame { pos_x: 8, pos_y: 4, width: 5, height: 5 });
        expect_eq!(fixture.first_difference(), None);
    }

    #[gtest]
    #[fuchsia::test]
    fn fill_padded_rows_with_pixel_stride_wider_than_pixel() {
        // 64-byte rows of 10 pixels give a pixel stride of 6 bytes for 4-byte pixels.
        let mut fixture = Fixture::new(10, 6, 4, 64);
        fixture.fill_region(&COLOR, Frame { pos_x: 0, pos_y: 0, width: 10, height: 6 });
        fixture.fill_region(&[1, 2, 3, 4], Frame { pos_x: 3, pos_y: 2, width: 4, height: 3 });
        fixture.fill_region(&[5, 6, 7, 8], Frame { pos_x: 8, pos_y: 5, width: 4, height: 4 });
        expect_eq!(fixture.first_difference(), None);
    }

    #[gtest]
    #[fuchsia::test]
    fn later_fills_paint_over_earlier_ones() {
        // A scaled-down multilayer-squares frame: clear, then overlapping squares.
        let mut fixture = Fixture::new(168, 374, 4, 168 * 4);
        fixture.fill_region(&[0, 0, 0, 0], Frame { pos_x: 0, pos_y: 0, width: 168, height: 374 });
        fixture.fill_region(&COLOR, Frame { pos_x: 10, pos_y: 20, width: 84, height: 84 });
        fixture
            .fill_region(&[59, 150, 0, 150], Frame { pos_x: 50, pos_y: 60, width: 21, height: 21 });
        fixture.fill_region(&[1, 2, 3, 4], Frame { pos_x: 120, pos_y: 300, width: 84, height: 84 });
        expect_eq!(fixture.first_difference(), None);
    }

    #[gtest]
    #[fuchsia::test]
    fn fill_with_wrong_color_size_fails_and_writes_nothing() {
        let fixture = Fixture::new(64, 48, 4, 256);
        let frame = Frame { pos_x: 0, pos_y: 0, width: 64, height: 48 };
        expect_true!(fixture.surface.fill_region(&[1, 2, 3], &frame).is_err());
        expect_true!(fixture.surface.fill_region(&[1, 2, 3, 4, 5], &frame).is_err());
        expect_eq!(fixture.first_difference(), None);
    }
}
