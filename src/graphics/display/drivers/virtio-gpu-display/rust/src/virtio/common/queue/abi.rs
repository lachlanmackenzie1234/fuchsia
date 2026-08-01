// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! The memory structures used by split virtqueues.

#[cfg(doc)]
use super::feature_bits::VirtioFeatureBits;

#[cfg(doc)]
use super::queue_buffer::{VirtioBufferRef, VirtioMemoryRangeAccess};

use bitfield::bitfield;
use std::num::Wrapping;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

bitfield! {
    /// [`VirtioMemoryRangeDescriptor`] properties.
    ///
    /// virtio14 2.7.5 "The Virtqueue Descriptor Table" > struct virtq_desc > flags
    #[repr(transparent)]
    #[derive(Copy, Clone, Default, FromBytes, IntoBytes, Immutable, KnownLayout)]
    pub struct VirtioMemoryRangeFlags(u16);
    impl Debug;

    /// True iff the buffer has at least one more range.
    ///
    /// Mutually exclusive with [`is_indirect`].
    ///
    /// virtio14 name: VIRTQ_DESC_F_NEXT
    pub bool, next_descriptor_index_is_valid, set_next_descriptor_index_is_valid: 0;

    /// Specifies device accesses to the range while its buffer is submitted.
    ///
    /// True (1) means [`VirtioMemoryRangeAccess::Output`]. False (0) means
    /// [`VirtioMemoryRangeAccess::Input`].
    ///
    /// virtio14 name: VIRTQ_DESC_F_WRITE
    pub bool, written_by_hardware, set_written_by_hardware: 1;

    /// True iff the descriptor is indirect.
    ///
    /// Mutually exclusive with [`next_descriptor_index_is_valid`]. Must only be
    /// set to true if [`VirtioFeatureBits::indirect_descriptors`] was
    /// negotiated.
    ///
    /// virtio14 2.7.5.3 "Indirect Descriptors" covers indirect descriptors. The
    /// memory area pointed by the indirect descriptor is a list of buffer
    /// descriptors, which cover the described buffer.
    ///
    /// This driver does not support indirect descriptors.
    ///
    /// virtio14 name: VIRTQ_DESC_F_INDIRECT
    pub bool, is_indirect, set_is_indirect: 2;
}

/// Describes a contiguous memory region belonging to a buffer in a virtqueue.
///
/// See [`VirtioBufferRef`] for a high-level description. Each descriptor
/// encodes a [`VirtioMemoryRange`]. Descriptors for memory regions belonging to
/// the same buffer are linked via
/// [`VirtioMemoryRangeDescriptor::next_descriptor_index`].
///
/// virtio14 2.7 "Split Virtqueues" documents the 16-byte alignment requirement.
///
/// virtio14 2.7.5 "The Virtqueue Descriptor Table" > struct virtq_desc
#[repr(C)]
#[derive(Copy, Clone, Debug, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct VirtioMemoryRangeDescriptor {
    /// Physical address of the memory region that contains buffer data.
    ///
    /// virtio14 name: addr
    pub physical_address: u64,

    /// The number of bytes in the memory region that contains buffer data.
    ///
    /// virtio14 name: len
    pub size_bytes: u32,

    /// Attributes that dictate the memory region's format and usage.
    ///
    /// virtio14 name: flags
    pub flags: VirtioMemoryRangeFlags,

    /// Points to the descriptor for the next memory region covering the buffer.
    ///
    /// The value is an index in the Descriptor Table.
    ///
    /// Only valid if [`VirtioMemoryRangeFlags::next_descriptor_index_is_valid`]
    /// is true (1).
    ///
    /// virtio14 name: next
    pub next_descriptor_index: u16,
}

/// The header of a split virtqueue's submitted ring.
///
/// Owned by the driver. The device will not modify the submitted ring.
///
/// virtio14 2.7.6 "The Virtqueue Available Ring" > struct virtq_avail
#[repr(C)]
#[derive(Copy, Clone, Debug, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct SubmittedRingHeader {
    /// Set by the driver.
    ///
    /// Must be zero (0) if
    /// [`VirtioFeatureBits::uses_virtqueue_notification_index`] was negotiated.
    ///
    /// If [`VirtioFeatureBits::uses_virtqueue_notification_index`] was not
    /// negotiated, setting to one (1) allows the device to skip issuing
    /// notifications for the associated virtqueue.
    ///
    /// This field is an optimization hint. The device is allowed to issue
    /// notifications for all virtqueue updates, and the driver must accept (and
    /// ignore) any spurious notifications.
    ///
    /// virtio14 2.7.7 "Used Buffer Notification Suppression" specifies the use
    /// of this field.
    pub flags: u16,

    /// Incremented every time the driver inserts an entry into the ring.
    ///
    /// The value modulo the ring capacity equals the index of the next ring
    /// entry to be populated by the driver.
    ///
    /// virtio14 name: idx
    pub insertion_counter: Wrapping<u16>,
}

/// An entry in a split virtqueue's submitted ring.
///
/// Owned by the driver. The device will not modify the submitted ring.
///
/// virtio14 2.7.6 "The Virtqueue Available Ring" > struct virtq_avail > ring
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct SubmittedRingEntry {
    /// Points to the first descriptor for the buffer submitted by the driver.
    ///
    /// Must be a valid index into the virtqueue's descriptor table.
    pub first_descriptor_index: u16,
}

/// The trailer of a split virtqueue's submitted ring.
///
/// Owned by the driver. The device will not modify the submitted ring.
///
/// virtio14 2.7.6 "The Virtqueue Available Ring" > struct virtq_avail
#[repr(C)]
#[derive(Copy, Clone, Debug, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct SubmittedRingTrailer {
    /// Points to the next [`ReturnedRingEntry`] that triggers a notification.
    ///
    /// virtio14 2.7.7.2 "Device Requirements: Used Buffer Notification
    /// Suppression" explains that this field is an optimization hint. The
    /// device is allowed to issue notifications for all virtqueue updates, and
    /// the driver must accept (and ignore) any spurious notifications.
    ///
    /// Valid iff [`VirtioFeatureBits::uses_virtqueue_notification_index`] was
    /// negotiated.
    ///
    /// virtio14 name: used_event
    pub next_notification_index: u16,
}

/// The header of a split virtqueue's returned ring.
///
/// Read-only. Only the device modifies the returned ring.
///
/// virtio14 2.7.8 "The Virtqueue Used Ring" > struct virtq_used
#[repr(C)]
#[derive(Copy, Clone, Debug, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct ReturnedRingHeader {
    /// Read by the driver before issuing a notification for the virtqueue.
    ///
    /// Must be zero (0) if
    /// [`VirtioFeatureBits::uses_virtqueue_notification_index`] was negotiated.
    ///
    /// If [`VirtioFeatureBits::uses_virtqueue_notification_index`] was not
    /// negotiated, setting to one (1) allows the driver to skip issuing
    /// notifications for the associated virtqueue.
    ///
    /// This field is an optimization hint. The driver is allowed to issue
    /// notifications for all virtqueue updates, and the device must accept (and
    /// ignore) any spurious notifications.
    ///
    /// virtio14 2.7.10 "Available Buffer Notification Suppression" specifies
    /// the use of this field.
    pub flags: u16,

    /// Incremented every time the device inserts an entry into the ring.
    ///
    /// The value modulo the ring capacity equals the index of the next ring
    /// entry to be populated by the device.
    ///
    /// virtio14 name: idx
    pub insertion_counter: Wrapping<u16>,
}

/// An entry in a split virtqueue's returned ring.
///
/// Read-only. Only the device modifies the returned ring.
///
/// virtio14 2.7.8 "The Virtqueue Used Ring" > struct virtq_used_elem
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct ReturnedRingEntry {
    /// Points to the first descriptor for the buffer returned by the device.
    ///
    /// Must equal [`SubmittedRingEntry::first_descriptor_index`] for a
    /// submitted ring entry previously produced by the driver.
    ///
    /// The maximum valid value fits in a [`u16`]. virtio14 states that the
    /// struct uses a [`u32`] for padding purposes.
    ///
    /// virtio14 name: id
    pub first_descriptor_index: u32,

    /// The number of bytes written by the device to the buffer.
    ///
    /// See [`VirtioReturnedBufferInfo::written_bytes`] for detailed semantics.
    ///
    /// virtio14 name: len
    pub written_bytes: u32,
}

/// The trailer of a split virtqueue's returned ring.
///
/// Read-only. Only the device modifies the returned ring.
///
/// virtio14 2.7.8 "The Virtqueue Used Ring" > struct virtq_used
#[repr(C)]
#[derive(Copy, Clone, Debug, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct ReturnedRingTrailer {
    /// Points to the next [`SubmittedRingEntry`] that triggers a notification.
    ///
    /// Valid iff [`VirtioFeatureBits::uses_virtqueue_notification_index`] was
    /// negotiated.
    ///
    /// virtio14 name: avail_event
    pub next_notification_index: u16,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, offset_of, size_of};

    #[fuchsia::test]
    fn test_virtio_submitted_ring_abi() {
        // virtio14 2.7 "Split Virtqueues" > table summarizing memory alignment
        // and size requirements.
        assert_eq!(align_of::<SubmittedRingHeader>(), 2);
        assert_eq!(align_of::<SubmittedRingEntry>(), 2);
        assert_eq!(align_of::<SubmittedRingTrailer>(), 2);
        assert_eq!(size_of::<SubmittedRingHeader>() + size_of::<SubmittedRingTrailer>(), 6);
        assert_eq!(size_of::<SubmittedRingEntry>(), 2);

        assert_eq!(size_of::<SubmittedRingHeader>(), 4);
        assert_eq!(size_of::<SubmittedRingTrailer>(), 2);
    }

    #[fuchsia::test]
    fn test_virtio_returned_ring_abi() {
        // virtio14 2.7 "Split Virtqueues" > table summarizing memory alignment
        // and size requirements.
        assert_eq!(align_of::<ReturnedRingHeader>(), 2);
        assert_eq!(align_of::<ReturnedRingEntry>(), 4);
        assert_eq!(align_of::<ReturnedRingTrailer>(), 2);
        assert_eq!(size_of::<ReturnedRingHeader>() + size_of::<ReturnedRingTrailer>(), 6);
        assert_eq!(size_of::<ReturnedRingEntry>(), 8);

        assert_eq!(size_of::<ReturnedRingHeader>(), 4);
        assert_eq!(size_of::<ReturnedRingTrailer>(), 2);
    }

    #[fuchsia::test]
    fn test_virtio_buffer_region_flags_abi() {
        assert_eq!(size_of::<VirtioMemoryRangeFlags>(), 2);
        assert_eq!(align_of::<VirtioMemoryRangeFlags>(), 2);
    }

    #[fuchsia::test]
    fn test_virtio_buffer_region_descriptor_abi() {
        // virtio14 2.7 "Split Virtqueues" > table summarizing memory alignment
        // and size requirements.
        assert_eq!(size_of::<VirtioMemoryRangeDescriptor>(), 16);
        assert_eq!(align_of::<VirtioMemoryRangeDescriptor>(), 8);

        assert_eq!(offset_of!(VirtioMemoryRangeDescriptor, physical_address), 0);
        assert_eq!(offset_of!(VirtioMemoryRangeDescriptor, size_bytes), 8);
        assert_eq!(offset_of!(VirtioMemoryRangeDescriptor, flags), 12);
        assert_eq!(offset_of!(VirtioMemoryRangeDescriptor, next_descriptor_index), 14);
    }
}
