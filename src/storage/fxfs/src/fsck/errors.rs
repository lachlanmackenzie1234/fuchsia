// Copyright 2021 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::log::*;
use crate::lsm_tree::types::ItemRef;
use crate::object_store::allocator::{AllocatorKey, AllocatorValue};
use crate::object_store::{AttributeId, ObjectDescriptor, ProjectId};
use fxfs_crypto::WrappingKeyId;
use std::ops::Range;

#[derive(Clone, Debug, PartialEq)]
pub enum FsckIssue {
    /// Warnings don't prevent the filesystem from mounting and don't fail fsck, but they indicate a
    /// consistency issue.
    Warning(FsckWarning),
    /// Errors prevent the filesystem from mounting, and will result in fsck failing, but will let
    /// fsck continue to run to find more issues.
    Error(FsckError),
    /// Fatal errors are like Errors, but they're serious enough that fsck should be halted, as any
    /// further results will probably be false positives.
    Fatal(FsckFatal),
}

impl FsckIssue {
    /// Translates an error to a human-readable string, intended for reporting errors to the user.
    /// For debugging, std::fmt::Debug is preferred.
    // TODO(https://fxbug.dev/42177349): Localization
    pub fn to_string(&self) -> String {
        match self {
            FsckIssue::Warning(w) => format!("WARNING: {}", w.to_string()),
            FsckIssue::Error(e) => format!("ERROR: {}", e.to_string()),
            FsckIssue::Fatal(f) => format!("FATAL: {}", f.to_string()),
        }
    }
    pub fn is_error(&self) -> bool {
        match self {
            FsckIssue::Error(_) | FsckIssue::Fatal(_) => true,
            FsckIssue::Warning(_) => false,
        }
    }
    pub fn log(&self) {
        match self {
            FsckIssue::Warning(w) => w.log(),
            FsckIssue::Error(e) => e.log(),
            FsckIssue::Fatal(f) => f.log(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)]
pub struct Allocation {
    range: Range<u64>,
    value: AllocatorValue,
}

impl From<ItemRef<'_, AllocatorKey, AllocatorValue>> for Allocation {
    fn from(item: ItemRef<'_, AllocatorKey, AllocatorValue>) -> Self {
        Self { range: (*item.key.device_range).clone(), value: item.value.clone() }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)]
pub struct Key(String);

impl<K: std::fmt::Debug, V> From<ItemRef<'_, K, V>> for Key {
    fn from(item: ItemRef<'_, K, V>) -> Self {
        Self(format!("{:?}", item.key))
    }
}

impl<K: std::fmt::Debug> From<&K> for Key {
    fn from(k: &K) -> Self {
        Self(format!("{:?}", k))
    }
}

#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)]
pub struct Value(String);

impl<K, V: std::fmt::Debug> From<ItemRef<'_, K, V>> for Value {
    fn from(item: ItemRef<'_, K, V>) -> Self {
        Self(format!("{:?}", item.value))
    }
}

// `From<V: std::fmt::Debug> for Value` creates a recursive definition since Value is Debug, so we
// have to go concrete here.
impl From<ObjectDescriptor> for Value {
    fn from(d: ObjectDescriptor) -> Self {
        Self(format!("{:?}", d))
    }
}

impl<V: std::fmt::Debug> From<&V> for Value {
    fn from(v: &V) -> Self {
        Self(format!("{:?}", v))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FsckWarning {
    ExtentForMissingAttribute(u64, u64, AttributeId),
    ExtentForNonexistentObject(u64, u64),
    GraveyardRecordForAbsentObject(u64, u64),
    InvalidObjectIdInStore(u64, Key, Value),
    LimitForNonExistentStore(u64, u64),
    OrphanedAttribute(u64, u64, AttributeId),
    OrphanedObject(u64, u64),
    OrphanedKeys(u64, u64),
    OrphanedExtendedAttribute(u64, u64, AttributeId),
    OrphanedExtendedAttributeRecord(u64, u64),
    ProjectUsageInconsistent(u64, ProjectId, (i64, i64), (i64, i64)),
}

impl FsckWarning {
    fn to_string(&self) -> String {
        match self {
            FsckWarning::ExtentForMissingAttribute(store_id, object_id, attr_id) => {
                format!(
                    "Found an extent in store {} for missing attribute {} on object {}",
                    store_id, attr_id, object_id
                )
            }
            FsckWarning::ExtentForNonexistentObject(store_id, object_id) => {
                format!(
                    "Found an extent in store {} for a non-existent object {}",
                    store_id, object_id
                )
            }
            FsckWarning::GraveyardRecordForAbsentObject(store_id, object_id) => {
                format!(
                    "Graveyard contains an entry for object {} in store {}, but that object is \
                    absent",
                    store_id, object_id
                )
            }
            FsckWarning::InvalidObjectIdInStore(store_id, key, value) => {
                format!("Store {} has an invalid object ID ({:?}, {:?})", store_id, key, value)
            }
            FsckWarning::LimitForNonExistentStore(store_id, limit) => {
                format!("Bytes limit of {} found for nonexistent store id {}", limit, store_id)
            }
            FsckWarning::OrphanedAttribute(store_id, object_id, attribute_id) => {
                format!(
                    "Attribute {} found for object {} which doesn't exist in store {}",
                    attribute_id, object_id, store_id
                )
            }
            FsckWarning::OrphanedObject(store_id, object_id) => {
                format!("Orphaned object {} was found in store {}", object_id, store_id)
            }
            FsckWarning::OrphanedKeys(store_id, object_id) => {
                format!("Orphaned keys for object {} were found in store {}", object_id, store_id)
            }
            FsckWarning::OrphanedExtendedAttribute(store_id, object_id, attribute_id) => {
                format!(
                    "Orphaned extended attribute for object {} was found in store {} with \
                    attribute id {}",
                    object_id, store_id, attribute_id,
                )
            }
            FsckWarning::OrphanedExtendedAttributeRecord(store_id, object_id) => {
                format!(
                    "Orphaned extended attribute record for object {} was found in store {}",
                    object_id, store_id
                )
            }
            FsckWarning::ProjectUsageInconsistent(store_id, project_id, stored, used) => {
                format!(
                    "Project id {} in store {} expected usage ({}, {}) found ({}, {})",
                    project_id, store_id, stored.0, stored.1, used.0, used.1
                )
            }
        }
    }

    fn log(&self) {
        match self {
            FsckWarning::ExtentForMissingAttribute(store_id, oid, attr_id) => {
                warn!(store_id, oid, attr_id; "Found an extent for a missing attribute");
            }
            FsckWarning::ExtentForNonexistentObject(store_id, oid) => {
                warn!(store_id, oid; "Extent for missing object");
            }
            FsckWarning::GraveyardRecordForAbsentObject(store_id, oid) => {
                warn!(store_id, oid; "Graveyard entry for missing object");
            }
            FsckWarning::InvalidObjectIdInStore(store_id, key, value) => {
                warn!(store_id, key:?, value:?; "Invalid object ID");
            }
            FsckWarning::LimitForNonExistentStore(store_id, limit) => {
                warn!(store_id, limit; "Found limit for non-existent owner store.");
            }
            FsckWarning::OrphanedAttribute(store_id, oid, attribute_id) => {
                warn!(store_id, oid, attribute_id; "Attribute for missing object");
            }
            FsckWarning::OrphanedObject(store_id, oid) => {
                warn!(oid, store_id; "Orphaned object");
            }
            FsckWarning::OrphanedKeys(store_id, oid) => {
                warn!(oid, store_id; "Orphaned keys");
            }
            FsckWarning::OrphanedExtendedAttribute(store_id, oid, attribute_id) => {
                warn!(oid, store_id, attribute_id; "Orphaned extended attribute");
            }
            FsckWarning::OrphanedExtendedAttributeRecord(store_id, oid) => {
                warn!(oid, store_id; "Orphaned extended attribute record");
            }
            FsckWarning::ProjectUsageInconsistent(store_id, project_id, stored, used) => {
                warn!(project_id, store_id, stored:?, used:?; "Project Inconsistent");
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FsckError {
    AllocatedBytesMismatch(Vec<(u64, u64)>, Vec<(u64, u64)>),
    AllocatedSizeMismatch(u64, u64, u64, u64),
    AllocationForNonexistentOwner(Allocation),
    AllocationMismatch(Allocation, Allocation),
    BadCasefoldHash(u64, u64, u64, u32, u32),
    BadGraveyardValue(u64, u64),
    BadLastObjectId(u64, u64),
    CasefoldInconsistency(u64, u64, u64),
    ChildEncryptedWithDifferentWrappingKeyThanParent(u64, u64, u64, WrappingKeyId, WrappingKeyId),
    ConflictingTypeForLink(u64, u64, Value, Value),
    DuplicateKey(u64, u64, u64),
    EncryptedChildDirectoryNoWrappingKey(u64, u64),
    EncryptedDirectoryHasUnencryptedChild(u64, u64, u64),
    ExtentExceedsLength(u64, u64, AttributeId, u64, Value),
    ExtraAllocations(Vec<Allocation>),
    IllegalKeyInRootStore(u64, u64),
    IncorrectMerkleTreeSize(u64, u64, u64, u64),
    LinkCycle(u64, u64),
    MalformedAllocation(Allocation),
    MalformedExtent(u64, u64, Range<u64>, u64),
    MalformedObjectRecord(u64, Key, Value),
    MisalignedAllocation(Allocation),
    MisalignedExtent(u64, u64, Range<u64>, u64),
    MissingAllocation(Allocation),
    InvalidExtendedAttributeId(u64, u64, AttributeId),
    MissingAttributeForExtendedAttribute(u64, u64, AttributeId),
    MissingDataAttribute(u64, u64),
    MissingEncryptionKeys(u64, u64),
    MissingKey(u64, u64, u64),
    MissingObjectInfo(u64, u64),
    MissingOverwriteExtents(u64, u64, AttributeId),
    MultipleLinksToDirectory(u64, u64),
    NextObjectIdInUse(u64, u64),
    NonFileMarkedAsVerified(u64, u64),
    NonRootProjectIdMetadata(u64, u64, ProjectId),
    ObjectCountMismatch(u64, u64, u64),
    ObjectHasChildren(u64, u64),
    OverwriteExtentFlagUnset(u64, u64, AttributeId),
    ProjectOnGraveyard(u64, ProjectId, u64),
    ProjectUsedWithNoUsageTracking(u64, ProjectId, u64),
    RefCountMismatch(u64, u64, u64),
    RootObjectHasParent(u64, u64, u64),
    SubDirCountMismatch(u64, u64, u64, u64),
    TombstonedAttributeDoesNotExist(u64, u64, AttributeId),
    TombstonedObjectHasRecords(u64, u64),
    TrimValueForGraveyardAttributeEntry(u64, u64, AttributeId),
    UnencryptedDirectoryHasEncryptedChild(u64, u64, u64),
    UnexpectedJournalFileOffset(u64),
    UnexpectedObjectInGraveyard(u64),
    UnexpectedRecordInObjectStore(u64, Key, Value),
    VerifiedFileDoesNotHaveAMerkleAttribute(u64, u64),
    VolumeInChildStore(u64, u64),
    ZombieDir(u64, u64, u64),
    ZombieFile(u64, u64, Vec<u64>),
    ZombieSymlink(u64, u64, Vec<u64>),
    InvalidInoLblk32KeyUsage(u64, u64),
}

impl FsckError {
    fn to_string(&self) -> String {
        match self {
            FsckError::AllocatedBytesMismatch(observed, stored) => {
                format!(
                    "Per-owner allocated bytes was {:?}, but sum of allocations gave {:?}",
                    stored, observed
                )
            }
            FsckError::AllocatedSizeMismatch(store_id, oid, observed, stored) => {
                format!(
                    "Expected {} bytes allocated for object {} in store {}, but found {} bytes",
                    stored, oid, store_id, observed
                )
            }
            FsckError::AllocationForNonexistentOwner(alloc) => {
                format!("Allocation {:?} for non-existent owner", alloc)
            }
            FsckError::AllocationMismatch(observed, stored) => {
                format!("Observed allocation {:?} but allocator has {:?}", observed, stored)
            }
            FsckError::BadCasefoldHash(store_id, parent_id, child_id, expected, actual) => {
                format!(
                    "Bad casefold hash code for store {store_id}, directory {parent_id}, child \
                     {child_id}. Expected {expected:08x}, actual {actual:08x}",
                )
            }
            FsckError::BadLastObjectId(highest, last_object_id) => {
                format!("Last object ID {last_object_id} is less than highest found {highest}")
            }
            FsckError::CasefoldInconsistency(store_id, parent_id, child_id) => {
                format!(
                    "CasefoldChild inconsistent for store {}, directory {}, child {}",
                    store_id, parent_id, child_id
                )
            }
            FsckError::ConflictingTypeForLink(store_id, object_id, expected, actual) => {
                format!(
                    "Object {} in store {} is of type {:?} but has a link of type {:?}",
                    store_id, object_id, expected, actual
                )
            }
            FsckError::ExtentExceedsLength(store_id, oid, attr_id, size, extent) => {
                format!(
                    "Extent {:?} exceeds length {} of attr {} on object {} in store {}",
                    extent, size, attr_id, oid, store_id
                )
            }
            FsckError::ExtraAllocations(allocations) => {
                format!("Unexpected allocations {:?}", allocations)
            }
            FsckError::IllegalKeyInRootStore(store_id, object_id) => {
                format!("Object {object_id} in root store {store_id} uses an illegal key type")
            }
            FsckError::ObjectHasChildren(store_id, object_id) => {
                format!("Object {} in store {} has unexpected children", object_id, store_id)
            }
            FsckError::UnexpectedJournalFileOffset(object_id) => {
                format!(
                    "SuperBlock journal_file_offsets contains unexpected object_id ({:?}).",
                    object_id
                )
            }
            FsckError::LinkCycle(store_id, object_id) => {
                format!("Detected cycle involving object {} in store {}", store_id, object_id)
            }
            FsckError::MalformedAllocation(allocations) => {
                format!("Malformed allocation {:?}", allocations)
            }
            FsckError::MalformedExtent(store_id, oid, extent, device_offset) => {
                format!(
                    "Extent {:?} (offset {}) for object {} in store {} is malformed",
                    extent, device_offset, oid, store_id
                )
            }
            FsckError::MalformedObjectRecord(store_id, key, value) => {
                format!(
                    "Object record in store {} has mismatched key {:?} and value {:?}",
                    store_id, key, value
                )
            }
            FsckError::MisalignedAllocation(allocations) => {
                format!("Misaligned allocation {:?}", allocations)
            }
            FsckError::MisalignedExtent(store_id, oid, extent, device_offset) => {
                format!(
                    "Extent {:?} (offset {}) for object {} in store {} is misaligned",
                    extent, device_offset, oid, store_id
                )
            }
            FsckError::MissingAllocation(allocation) => {
                format!("Observed {:?} but didn't find record in allocator", allocation)
            }
            FsckError::InvalidExtendedAttributeId(store_id, oid, attribute_id) => {
                format!(
                    "Object {} in store {} has an extended attribute stored in an invalid \
                    attribute {}",
                    oid, store_id, attribute_id
                )
            }
            FsckError::MissingAttributeForExtendedAttribute(store_id, oid, attribute_id) => {
                format!(
                    "Object {} in store {} has an extended attribute stored in a nonexistent \
                    attribute {}",
                    store_id, oid, attribute_id
                )
            }
            FsckError::MissingDataAttribute(store_id, oid) => {
                format!("File {} in store {} didn't have the default data attribute", store_id, oid)
            }
            FsckError::MissingObjectInfo(store_id, object_id) => {
                format!("Object {} in store {} had no object record", store_id, object_id)
            }
            FsckError::MultipleLinksToDirectory(store_id, object_id) => {
                format!("Directory {} in store {} has multiple links", store_id, object_id)
            }
            FsckError::NonRootProjectIdMetadata(store_id, object_id, project_id) => {
                format!(
                    "Project Id {} metadata in store {} attached to object {}",
                    project_id, store_id, object_id
                )
            }
            FsckError::ObjectCountMismatch(store_id, observed, stored) => {
                format!("Store {} had {} objects, expected {}", store_id, observed, stored)
            }
            FsckError::ProjectOnGraveyard(store_id, project_id, object_id) => {
                format!(
                    "Store {} had graveyard object {} with project id {}",
                    store_id, object_id, project_id
                )
            }
            FsckError::ProjectUsedWithNoUsageTracking(store_id, project_id, node_id) => {
                format!(
                    "Store {} had node {} with project ids {} but no usage tracking metadata",
                    store_id, node_id, project_id
                )
            }
            FsckError::RefCountMismatch(oid, observed, stored) => {
                format!("Object {} had {} references, expected {}", oid, observed, stored)
            }
            FsckError::RootObjectHasParent(store_id, object_id, apparent_parent_id) => {
                format!(
                    "Object {} is child of {} but is a root object of store {}",
                    object_id, apparent_parent_id, store_id
                )
            }
            FsckError::SubDirCountMismatch(store_id, object_id, observed, stored) => {
                format!(
                    "Directory {} in store {} should have {} sub dirs but had {}",
                    object_id, store_id, stored, observed
                )
            }
            FsckError::TombstonedObjectHasRecords(store_id, object_id) => {
                format!(
                    "Tombstoned object {} in store {} was referenced by other records",
                    store_id, object_id
                )
            }
            FsckError::UnexpectedObjectInGraveyard(object_id) => {
                format!("Found a non-file object {} in graveyard", object_id)
            }
            FsckError::UnexpectedRecordInObjectStore(store_id, key, value) => {
                format!("Unexpected record ({:?}, {:?}) in object store {}", key, value, store_id)
            }
            FsckError::VolumeInChildStore(store_id, object_id) => {
                format!(
                    "Volume {} found in child store {} instead of root store",
                    object_id, store_id
                )
            }
            FsckError::BadGraveyardValue(store_id, object_id) => {
                format!("Bad graveyard value with key <{}, {}>", store_id, object_id)
            }
            FsckError::MissingEncryptionKeys(store_id, object_id) => {
                format!("Missing encryption keys for <{}, {}>", store_id, object_id)
            }
            FsckError::MissingKey(store_id, object_id, key_id) => {
                format!("Missing encryption key for <{}, {}, {}>", store_id, object_id, key_id)
            }
            FsckError::EncryptedChildDirectoryNoWrappingKey(store_id, object_id) => {
                format!(
                    "Encrypted directory {} in store {} does not have a wrapping key id set",
                    object_id, store_id
                )
            }
            FsckError::EncryptedDirectoryHasUnencryptedChild(store_id, parent_oid, child_oid) => {
                format!(
                    "Encrypted parent directory {} in store {} has unencrypted child {}",
                    parent_oid, store_id, child_oid
                )
            }
            FsckError::UnencryptedDirectoryHasEncryptedChild(store_id, parent_oid, child_oid) => {
                format!(
                    "Unencrypted parent directory {} in store {} has encrypted child {}",
                    parent_oid, store_id, child_oid
                )
            }
            FsckError::ChildEncryptedWithDifferentWrappingKeyThanParent(
                store_id,
                parent_id,
                child_id,
                parent_wrapping_key_id,
                child_wrapping_key_id,
            ) => {
                format!(
                    "Parent directory {} in store {} encrypted with {:?}, child {} encrypted with \
                    {:?}",
                    parent_id, store_id, parent_wrapping_key_id, child_id, child_wrapping_key_id,
                )
            }
            FsckError::DuplicateKey(store_id, object_id, key_id) => {
                format!("Duplicate key for <{}, {}, {}>", store_id, object_id, key_id)
            }
            FsckError::ZombieFile(store_id, object_id, parent_object_ids) => {
                format!(
                    "File {object_id} in store {store_id} is in graveyard but still has links \
                     from {parent_object_ids:?}",
                )
            }
            FsckError::ZombieDir(store_id, object_id, parent_object_id) => {
                format!(
                    "Directory {object_id} in store {store_id} is in graveyard but still has \
                     a link from {parent_object_id}",
                )
            }
            FsckError::ZombieSymlink(store_id, object_id, parent_object_ids) => {
                format!(
                    "Symlink {object_id} in store {store_id} is in graveyard but still has \
                     links from {parent_object_ids:?}",
                )
            }
            FsckError::VerifiedFileDoesNotHaveAMerkleAttribute(store_id, object_id) => {
                format!(
                    "Object {} in store {} is marked as fsverity-enabled but is missing a \
                        merkle attribute",
                    store_id, object_id
                )
            }
            FsckError::NonFileMarkedAsVerified(store_id, object_id) => {
                format!(
                    "Object {} in store {} is marked as verified but is not a file",
                    store_id, object_id
                )
            }
            FsckError::InvalidInoLblk32KeyUsage(store_id, object_id) => {
                format!("Object {object_id} in store {store_id} uses an InoLblk32 key invalidly")
            }
            FsckError::IncorrectMerkleTreeSize(store_id, object_id, expected_size, actual_size) => {
                format!(
                    "Object {} in store {} has merkle tree of size {} expected {}",
                    object_id, store_id, actual_size, expected_size
                )
            }
            FsckError::TombstonedAttributeDoesNotExist(store_id, object_id, attribute_id) => {
                format!(
                    "Object {} in store {} has an attribute {} that is tombstoned but does not \
                     exist.",
                    object_id, store_id, attribute_id
                )
            }
            FsckError::TrimValueForGraveyardAttributeEntry(store_id, object_id, attribute_id) => {
                format!(
                    "Object {} in store {} has a GraveyardAttributeEntry for attribute {} that has \
                     ObjectValue::Trim",
                    object_id, store_id, attribute_id,
                )
            }
            FsckError::MissingOverwriteExtents(store_id, object_id, attribute_id) => {
                format!(
                    "Object {} in store {} has an attribute {} that indicated it had overwrite \
                     extents but none were found",
                    object_id, store_id, attribute_id,
                )
            }
            FsckError::OverwriteExtentFlagUnset(store_id, object_id, attribute_id) => {
                format!(
                    "Object {} in store {} has an attribute {} with overwrite extents but the \
                     metadata indicated it would not",
                    object_id, store_id, attribute_id,
                )
            }
            FsckError::NextObjectIdInUse(store_id, next_object_id) => {
                format!("Next object ID {store_id} will use ({next_object_id}) is already in use",)
            }
        }
    }

    fn log(&self) {
        match self {
            FsckError::AllocatedBytesMismatch(observed, stored) => {
                error!(observed:?, stored:?; "Unexpected allocated bytes");
            }
            FsckError::AllocatedSizeMismatch(store_id, oid, observed, stored) => {
                error!(observed, oid, store_id, stored; "Unexpected allocated size");
            }
            FsckError::AllocationForNonexistentOwner(alloc) => {
                error!(alloc:?; "Allocation for non-existent owner")
            }
            FsckError::AllocationMismatch(observed, stored) => {
                error!(observed:?, stored:?; "Unexpected allocation");
            }
            FsckError::BadCasefoldHash(store_id, parent_id, child_id, expected, actual) => {
                warn!(store_id, parent_id, child_id, expected, actual; "Bad casefold hash code");
            }
            FsckError::BadLastObjectId(highest, last_object_id) => {
                error!(highest, last_object_id; "Last object ID is less than highest found");
            }
            FsckError::CasefoldInconsistency(store_id, parent_id, child_id) => {
                error!(store_id:?, parent_id:?, child_id:?; "CasefoldChild inconsistent");
            }
            FsckError::ConflictingTypeForLink(store_id, oid, expected, actual) => {
                error!(store_id, oid, expected:?, actual:?; "Bad link");
            }
            FsckError::ExtentExceedsLength(store_id, oid, attr_id, size, extent) => {
                error!(store_id, oid, attr_id, size, extent:?; "Extent exceeds length");
            }
            FsckError::ExtraAllocations(allocations) => {
                error!(allocations:?; "Unexpected allocations");
            }
            FsckError::IllegalKeyInRootStore(store_id, oid) => {
                error!(store_id, oid; "Illegal key in root store");
            }
            FsckError::ObjectHasChildren(store_id, oid) => {
                error!(store_id, oid; "Object has unexpected children");
            }
            FsckError::UnexpectedJournalFileOffset(object_id) => {
                error!(
                    oid = object_id;
                    "SuperBlock journal_file_offsets contains unexpected object-id"
                );
            }
            FsckError::LinkCycle(store_id, oid) => {
                error!(store_id, oid; "Link cycle");
            }
            FsckError::MalformedAllocation(allocations) => {
                error!(allocations:?; "Malformed allocations");
            }
            FsckError::MalformedExtent(store_id, oid, extent, device_offset) => {
                error!(store_id, oid, extent:?, device_offset; "Malformed extent");
            }
            FsckError::MalformedObjectRecord(store_id, key, value) => {
                error!(store_id, key:?, value:?; "Mismatched key and value");
            }
            FsckError::MisalignedAllocation(allocations) => {
                error!(allocations:?; "Misaligned allocation");
            }
            FsckError::MisalignedExtent(store_id, oid, extent, device_offset) => {
                error!(store_id, oid, extent:?, device_offset; "Misaligned extent");
            }
            FsckError::MissingAllocation(allocation) => {
                error!(allocation:?; "Missing allocation");
            }
            FsckError::InvalidExtendedAttributeId(store_id, oid, attribute_id) => {
                error!(store_id, oid, attribute_id; "Invalid extended attribute id");
            }
            FsckError::MissingAttributeForExtendedAttribute(store_id, oid, attribute_id) => {
                error!(store_id, oid, attribute_id; "Missing attribute for extended attribute");
            }
            FsckError::MissingDataAttribute(store_id, oid) => {
                error!(store_id, oid; "Missing default attribute");
            }
            FsckError::MissingObjectInfo(store_id, oid) => {
                error!(store_id, oid; "Missing object record");
            }
            FsckError::MultipleLinksToDirectory(store_id, oid) => {
                error!(store_id, oid; "Directory with multiple links");
            }
            FsckError::NonRootProjectIdMetadata(store_id, object_id, project_id) => {
                error!(
                    store_id,
                    object_id, project_id; "Non root object in volume with project id metadata"
                );
            }
            FsckError::ObjectCountMismatch(store_id, observed, stored) => {
                error!(store_id, observed, stored; "Object count mismatch");
            }
            FsckError::ProjectOnGraveyard(store_id, project_id, object_id) => {
                error!(store_id, project_id, object_id; "Project was set on graveyard object");
            }
            FsckError::ProjectUsedWithNoUsageTracking(store_id, project_id, node_id) => {
                error!(store_id, project_id, node_id; "Project used without tracking metadata");
            }
            FsckError::RefCountMismatch(oid, observed, stored) => {
                error!(oid, observed, stored; "Reference count mismatch");
            }
            FsckError::RootObjectHasParent(store_id, oid, apparent_parent_id) => {
                error!(store_id, oid, apparent_parent_id; "Root object is a child");
            }
            FsckError::SubDirCountMismatch(store_id, oid, observed, stored) => {
                error!(store_id, oid, observed, stored; "Sub-dir count mismatch");
            }
            FsckError::TombstonedObjectHasRecords(store_id, oid) => {
                error!(store_id, oid; "Tombstoned object with references");
            }
            FsckError::UnexpectedObjectInGraveyard(oid) => {
                error!(oid; "Unexpected object in graveyard");
            }
            FsckError::UnexpectedRecordInObjectStore(store_id, key, value) => {
                error!(store_id, key:?, value:?; "Unexpected record");
            }
            FsckError::VolumeInChildStore(store_id, oid) => {
                error!(store_id, oid; "Volume in child store");
            }
            FsckError::BadGraveyardValue(store_id, oid) => {
                error!(store_id, oid; "Bad graveyard value");
            }
            FsckError::MissingEncryptionKeys(store_id, oid) => {
                error!(store_id, oid; "Missing encryption keys");
            }
            FsckError::MissingKey(store_id, oid, key_id) => {
                error!(store_id, oid, key_id; "Missing encryption key");
            }
            FsckError::EncryptedChildDirectoryNoWrappingKey(store_id, oid) => {
                error!(store_id, oid; "Encrypted directory does not have a wrapping key id");
            }
            FsckError::EncryptedDirectoryHasUnencryptedChild(store_id, parent_oid, child_oid) => {
                error!(
                    store_id,
                    parent_oid, child_oid; "Encrypted directory has unencrypted child"
                );
            }
            FsckError::UnencryptedDirectoryHasEncryptedChild(store_id, parent_oid, child_oid) => {
                error!(
                    store_id,
                    parent_oid, child_oid; "Unencrypted directory has encrypted child"
                );
            }
            FsckError::ChildEncryptedWithDifferentWrappingKeyThanParent(
                store_id,
                parent_id,
                child_id,
                parent_wrapping_key_id,
                child_wrapping_key_id,
            ) => {
                error!(
                    store_id,
                    parent_id,
                    child_id,
                    parent_wrapping_key_id:?,
                    child_wrapping_key_id:?;
                    "Child object encrypted with different wrapping key than parent"
                );
            }
            FsckError::DuplicateKey(store_id, oid, key_id) => {
                error!(store_id, oid, key_id; "Duplicate key")
            }
            FsckError::ZombieFile(store_id, oid, parent_oids) => {
                error!(store_id, oid, parent_oids:?; "Links exist to file in graveyard")
            }
            FsckError::ZombieDir(store_id, oid, parent_oid) => {
                error!(store_id, oid, parent_oid; "A link exists to directory in graveyard")
            }
            FsckError::ZombieSymlink(store_id, oid, parent_oids) => {
                error!(store_id, oid, parent_oids:?; "Links exists to symlink in graveyard")
            }
            FsckError::VerifiedFileDoesNotHaveAMerkleAttribute(store_id, oid) => {
                error!(store_id, oid; "Verified file does not have a merkle attribute")
            }
            FsckError::NonFileMarkedAsVerified(store_id, oid) => {
                error!(store_id, oid; "Non-file marked as verified")
            }
            FsckError::InvalidInoLblk32KeyUsage(store_id, oid) => {
                error!(store_id, oid; "Invalid InoLblk32 key usage")
            }
            FsckError::IncorrectMerkleTreeSize(store_id, oid, expected_size, actual_size) => {
                error!(
                    store_id,
                    oid, expected_size, actual_size; "Verified file has incorrect merkle tree size"
                )
            }
            FsckError::TombstonedAttributeDoesNotExist(store_id, oid, attribute_id) => {
                error!(store_id, oid, attribute_id; "Tombstoned attribute does not exist")
            }
            FsckError::TrimValueForGraveyardAttributeEntry(store_id, oid, attribute_id) => {
                error!(
                    store_id,
                    oid, attribute_id; "Invalid Trim value for a graveyard attribute entry",
                )
            }
            FsckError::MissingOverwriteExtents(store_id, oid, attribute_id) => {
                error!(
                    store_id,
                    oid,
                    attribute_id;
                    "Overwrite extents indicated, but no overwrite extents were found",
                )
            }
            FsckError::OverwriteExtentFlagUnset(store_id, oid, attribute_id) => {
                error!(
                    store_id,
                    oid,
                    attribute_id;
                    "Overwrite extents were found, but metadata flag was not set",
                )
            }
            FsckError::NextObjectIdInUse(store_id, next_object_id) => {
                error!(store_id, next_object_id; "Next object ID is already in use");
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FsckFatal {
    MalformedGraveyard,
    MalformedLayerFile(u64, u64),
    MalformedStore(u64),
    MisOrderedLayerFile(u64, u64),
    MisOrderedObjectStore(u64),
    OverlappingKeysInLayerFile(u64, u64, Key, Key),
    InvalidBloomFilter(u64, u64, Key),
}

impl FsckFatal {
    fn to_string(&self) -> String {
        match self {
            FsckFatal::MalformedGraveyard => {
                "Graveyard is malformed; root store is inconsistent".to_string()
            }
            FsckFatal::MalformedLayerFile(store_id, layer_file_id) => {
                format!("Layer file {} in object store {} is malformed", layer_file_id, store_id)
            }
            FsckFatal::MalformedStore(id) => {
                format!("Object store {} is malformed; root store is inconsistent", id)
            }
            FsckFatal::MisOrderedLayerFile(store_id, layer_file_id) => {
                format!(
                    "Layer file {} for store/allocator {} contains out-of-order records",
                    layer_file_id, store_id
                )
            }
            FsckFatal::MisOrderedObjectStore(store_id) => {
                format!("Store/allocator {} contains out-of-order or duplicate records", store_id)
            }
            FsckFatal::OverlappingKeysInLayerFile(store_id, layer_file_id, key1, key2) => {
                format!(
                    "Layer file {} for store/allocator {} contains overlapping keys {:?} and {:?}",
                    layer_file_id, store_id, key1, key2
                )
            }
            FsckFatal::InvalidBloomFilter(store_id, layer_file_id, key) => {
                format!(
                    "Filter for layer files is invalid: reported that key {:?} in layer file {} \
                    for store/allocator {} does not exist",
                    key, layer_file_id, store_id
                )
            }
        }
    }

    fn log(&self) {
        match self {
            FsckFatal::MalformedGraveyard => {
                error!("Graveyard is malformed; root store is inconsistent");
            }
            FsckFatal::MalformedLayerFile(store_id, layer_file_id) => {
                error!(store_id, layer_file_id; "Layer file malformed");
            }
            FsckFatal::MalformedStore(id) => {
                error!(id; "Malformed store; root store is inconsistent");
            }
            FsckFatal::MisOrderedLayerFile(store_id, layer_file_id) => {
                // This can be for stores or the allocator.
                error!(oid = store_id, layer_file_id; "Layer file contains out-of-oder records");
            }
            FsckFatal::MisOrderedObjectStore(store_id) => {
                // This can be for stores or the allocator.
                error!(
                    oid = store_id;
                    "Store/allocator contains out-of-order or duplicate records"
                );
            }
            FsckFatal::OverlappingKeysInLayerFile(store_id, layer_file_id, key1, key2) => {
                // This can be for stores or the allocator.
                error!(oid = store_id, layer_file_id, key1:?, key2:?; "Overlapping keys");
            }
            FsckFatal::InvalidBloomFilter(store_id, layer_file_id, key) => {
                error!(oid = store_id, layer_file_id, key:?; "Filter for layer files invalid");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsm_tree::types::Item;
    use crate::object_store::ObjectDescriptor;

    #[fuchsia::test]
    fn test_type_conversions() {
        let alloc_item = Item::new(
            AllocatorKey { device_range: (0..4096).into() },
            AllocatorValue::Abs { count: 1, owner_object_id: 2 },
        );
        let alloc = Allocation::from(alloc_item.as_item_ref());
        assert_eq!(alloc.range, 0..4096);
        assert_eq!(alloc.value, AllocatorValue::Abs { count: 1, owner_object_id: 2 });

        let key_from_item = Key::from(alloc_item.as_item_ref());
        let key_from_ref = Key::from(&alloc_item.key);
        assert_eq!(key_from_item, key_from_ref);

        let val_from_item = Value::from(alloc_item.as_item_ref());
        let val_from_ref = Value::from(&alloc_item.value);
        assert_eq!(val_from_item, val_from_ref);

        let val_from_desc = Value::from(ObjectDescriptor::File);
        assert_eq!(val_from_desc, Value::from(&ObjectDescriptor::File));
    }

    #[fuchsia::test]
    fn test_fsck_issue() {
        let warning = FsckIssue::Warning(FsckWarning::OrphanedObject(1, 2));
        assert!(!warning.is_error());
        assert!(warning.to_string().starts_with("WARNING: "));
        warning.log();

        let err = FsckIssue::Error(FsckError::UnexpectedObjectInGraveyard(3));
        assert!(err.is_error());
        assert!(err.to_string().starts_with("ERROR: "));
        err.log();

        let fatal = FsckIssue::Fatal(FsckFatal::MalformedGraveyard);
        assert!(fatal.is_error());
        assert!(fatal.to_string().starts_with("FATAL: "));
        fatal.log();
    }

    #[fuchsia::test]
    fn test_fsck_warnings() {
        let k = Key::from(&1u64);
        let v = Value::from(&2u64);
        let attr_id = AttributeId(3);
        let proj_id = ProjectId::new(2).unwrap();
        let warnings = vec![
            FsckWarning::ExtentForMissingAttribute(1, 2, attr_id),
            FsckWarning::ExtentForNonexistentObject(1, 2),
            FsckWarning::GraveyardRecordForAbsentObject(1, 2),
            FsckWarning::InvalidObjectIdInStore(1, k, v),
            FsckWarning::LimitForNonExistentStore(1, 100),
            FsckWarning::OrphanedAttribute(1, 2, attr_id),
            FsckWarning::OrphanedObject(1, 2),
            FsckWarning::OrphanedKeys(1, 2),
            FsckWarning::OrphanedExtendedAttribute(1, 2, attr_id),
            FsckWarning::OrphanedExtendedAttributeRecord(1, 2),
            FsckWarning::ProjectUsageInconsistent(1, proj_id, (10, 1), (20, 2)),
        ];
        for warning in warnings {
            assert!(!warning.to_string().is_empty());
            warning.log();
        }
    }

    #[fuchsia::test]
    fn test_fsck_errors() {
        let alloc = Allocation {
            range: 0..4096,
            value: AllocatorValue::Abs { count: 1, owner_object_id: 2 },
        };
        let k = Key::from(&1u64);
        let v = Value::from(&2u64);
        let attr_id = AttributeId(3);
        let proj_id = ProjectId::new(2).unwrap();
        let wk1: WrappingKeyId = u128::to_le_bytes(1);
        let wk2: WrappingKeyId = u128::to_le_bytes(2);
        let errors = vec![
            FsckError::AllocatedBytesMismatch(vec![(1, 100)], vec![(1, 200)]),
            FsckError::AllocatedSizeMismatch(1, 2, 100, 200),
            FsckError::AllocationForNonexistentOwner(alloc.clone()),
            FsckError::AllocationMismatch(alloc.clone(), alloc.clone()),
            FsckError::BadCasefoldHash(1, 2, 3, 4, 5),
            FsckError::BadGraveyardValue(1, 2),
            FsckError::BadLastObjectId(10, 5),
            FsckError::CasefoldInconsistency(1, 2, 3),
            FsckError::ChildEncryptedWithDifferentWrappingKeyThanParent(1, 2, 3, wk1, wk2),
            FsckError::ConflictingTypeForLink(1, 2, v.clone(), v.clone()),
            FsckError::DuplicateKey(1, 2, 3),
            FsckError::EncryptedChildDirectoryNoWrappingKey(1, 2),
            FsckError::EncryptedDirectoryHasUnencryptedChild(1, 2, 3),
            FsckError::ExtentExceedsLength(1, 2, attr_id, 100, v.clone()),
            FsckError::ExtraAllocations(vec![alloc.clone()]),
            FsckError::IllegalKeyInRootStore(1, 2),
            FsckError::IncorrectMerkleTreeSize(1, 2, 100, 200),
            FsckError::LinkCycle(1, 2),
            FsckError::MalformedAllocation(alloc.clone()),
            FsckError::MalformedExtent(1, 2, 0..4096, 8192),
            FsckError::MalformedObjectRecord(1, k.clone(), v.clone()),
            FsckError::MisalignedAllocation(alloc.clone()),
            FsckError::MisalignedExtent(1, 2, 0..4096, 8192),
            FsckError::MissingAllocation(alloc),
            FsckError::InvalidExtendedAttributeId(1, 2, attr_id),
            FsckError::MissingAttributeForExtendedAttribute(1, 2, attr_id),
            FsckError::MissingDataAttribute(1, 2),
            FsckError::MissingEncryptionKeys(1, 2),
            FsckError::MissingKey(1, 2, 3),
            FsckError::MissingObjectInfo(1, 2),
            FsckError::MissingOverwriteExtents(1, 2, attr_id),
            FsckError::MultipleLinksToDirectory(1, 2),
            FsckError::NextObjectIdInUse(1, 2),
            FsckError::NonFileMarkedAsVerified(1, 2),
            FsckError::NonRootProjectIdMetadata(1, 2, proj_id),
            FsckError::ObjectCountMismatch(1, 2, 3),
            FsckError::ObjectHasChildren(1, 2),
            FsckError::OverwriteExtentFlagUnset(1, 2, attr_id),
            FsckError::ProjectOnGraveyard(1, proj_id, 3),
            FsckError::ProjectUsedWithNoUsageTracking(1, proj_id, 3),
            FsckError::RefCountMismatch(1, 2, 3),
            FsckError::RootObjectHasParent(1, 2, 3),
            FsckError::SubDirCountMismatch(1, 2, 3, 4),
            FsckError::TombstonedAttributeDoesNotExist(1, 2, attr_id),
            FsckError::TombstonedObjectHasRecords(1, 2),
            FsckError::TrimValueForGraveyardAttributeEntry(1, 2, attr_id),
            FsckError::UnencryptedDirectoryHasEncryptedChild(1, 2, 3),
            FsckError::UnexpectedJournalFileOffset(1),
            FsckError::UnexpectedObjectInGraveyard(1),
            FsckError::UnexpectedRecordInObjectStore(1, k, v),
            FsckError::VerifiedFileDoesNotHaveAMerkleAttribute(1, 2),
            FsckError::VolumeInChildStore(1, 2),
            FsckError::ZombieDir(1, 2, 3),
            FsckError::ZombieFile(1, 2, vec![3]),
            FsckError::ZombieSymlink(1, 2, vec![3]),
            FsckError::InvalidInoLblk32KeyUsage(1, 2),
        ];
        for err in errors {
            assert!(!err.to_string().is_empty());
            err.log();
        }
    }

    #[fuchsia::test]
    fn test_fsck_fatals() {
        let k = Key::from(&1u64);
        let fatals = vec![
            FsckFatal::MalformedGraveyard,
            FsckFatal::MalformedLayerFile(1, 2),
            FsckFatal::MalformedStore(1),
            FsckFatal::MisOrderedLayerFile(1, 2),
            FsckFatal::MisOrderedObjectStore(1),
            FsckFatal::OverlappingKeysInLayerFile(1, 2, k.clone(), k.clone()),
            FsckFatal::InvalidBloomFilter(1, 2, k),
        ];
        for fatal in fatals {
            assert!(!fatal.to_string().is_empty());
            fatal.log();
        }
    }
}
