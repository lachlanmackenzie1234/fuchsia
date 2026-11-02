// Copyright 2016 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include <assert.h>
#include <inttypes.h>
#include <lib/counters.h>
#include <lib/page/size.h>
#include <trace.h>
#include <zircon/errors.h>
#include <zircon/types.h>

#include <ktl/algorithm.h>
#include <ktl/utility.h>
#include <vm/vm_address_region.h>
#include <vm/vm_aspace.h>
#include <vm/vm_object.h>
#include <vm/vm_object_paged.h>

#include "vm_priv.h"

#include <ktl/enforce.h>

#define LOCAL_TRACE VM_GLOBAL_TRACE(0)

namespace {

KCOUNTER(vm_mapping_attribution_queries, "vm.attributed_memory.mapping.queries")
KCOUNTER(vm_mappings_merged, "vm.aspace.mapping.merged_neighbors")

}  // namespace

VmMapping::VmMapping(VmAddressRegion& parent, bool private_clone, vaddr_t base, size_t size,
                     uint32_t vmar_flags, fbl::RefPtr<VmObject> vmo, uint64_t vmo_offset,
                     arch_mmu_flags_t first_mmu_flags,
                     btree::BTree<vaddr_t, arch_mmu_flags_t>&& ranges, Mergeable mergeable)
    : VmAddressRegionOrMapping(base, size, vmar_flags, parent.aspace().get(), &parent, true),
      rest_protection_ranges_(ktl::move(ranges)) {
  rust_vm_mapping_state_init(&opaque_storage_, fbl::ExportToRawPtr(&vmo), vmo_offset,
                             first_mmu_flags, private_clone, mergeable);
  LTRACEF("%p aspace %p base %#" PRIxPTR " size %#zx offset %#" PRIx64 "\n", this, aspace().get(),
          this->base(), this->size(), vmo_offset);
}

VmMapping::VmMapping(VmAddressRegion& parent, bool private_clone, vaddr_t base, size_t size,
                     uint32_t vmar_flags, fbl::RefPtr<VmObject> vmo, uint64_t vmo_offset,
                     arch_mmu_flags_t arch_mmu_flags, Mergeable mergeable)
    : VmMapping(parent, private_clone, base, size, vmar_flags, ktl::move(vmo), vmo_offset,
                arch_mmu_flags, btree::BTree<vaddr_t, arch_mmu_flags_t>(), mergeable) {}

VmMapping::~VmMapping() { rust_vm_mapping_destruct(this); }

void VmMapping::fbl_recycle() { rust_vm_mapping_recycle(this); }

fbl::RefPtr<VmObject> VmMapping::vmo() const {
  return fbl::ImportFromRawPtr(rust_vm_mapping_vmo(this));
}

VmMapping::AttributionCounts VmMapping::GetAttributedMemoryLocked(
    Guard<CriticalMutex>& guard) const {
  AssertCanary();

  if (!IsAliveLocked()) {
    return AttributionCounts{};
  }

  vm_mapping_attribution_queries.Add(1);

  fbl::RefPtr<VmObject> vmo = object_locked();
  const uint64_t object_offset = this->object_offset();
  VmMapping::AttributionCounts page_counts;
  guard.CallUnlocked(
      [&]() { page_counts = vmo->GetAttributedMemoryInRange(object_offset, size()); });
  return page_counts;
}

VmMapping::AttributionCounts VmMapping::GetAttributedMemory() const {
  AssertCanary();

  Guard<CriticalMutex> guard{lock()};
  return GetAttributedMemoryLocked(guard);
}

zx_status_t VmMapping::UnmapLocked(vaddr_t base, size_t size) {
  AssertCanary();
  DEBUG_ASSERT(size != 0 && IsPageRounded(size) && IsPageRounded(base));
  DEBUG_ASSERT(base >= this->base() && base - this->base() < this->size());
  DEBUG_ASSERT(this->size() - (base - this->base()) >= size);
  DEBUG_ASSERT(parent_locked());

  // Take a ref to ourselves in case we drop the last one when removing from our parent.
  fbl::RefPtr<VmMapping> self(this);

  if (state_locked() != LifeCycleState::ALIVE) {
    return ZX_ERR_BAD_STATE;
  }

  AssertHeld(parent_locked()->lock_ref());
  AssertHeld(parent_locked()->region_lock_ref());

  // Should never be unmapping everything, otherwise should destroy.
  DEBUG_ASSERT(base != this->base() || size != this->size());

  LTRACEF("%p\n", this);

  // First create any new mapping. One or two might be required depending on whether unmapping from
  // an end or the middle.
  fbl::RefPtr<VmMapping> left, right;
  if (this->base() != base) {
    fbl::RefPtr<VmObject> vmo = object_locked();
    // Insert empty protection ranges for now as we will transform the existing protection_ranges_
    // tree into the target.
    btree::BTree<vaddr_t, arch_mmu_flags_t> left_prot_ranges;
    left = fbl::ImportFromRawPtr(rust_vm_mapping_create_with_ranges(
        parent_locked(), private_clone(), this->base(), base - this->base(), flags(),
        fbl::ExportToRawPtr(&vmo), object_offset(), 0, &left_prot_ranges, Mergeable::YES));
    if (!left) {
      return ZX_ERR_NO_MEMORY;
      AssertHeld(parent_locked()->region_lock_ref());
    }
  }
  if (base + size != this->base() + this->size()) {
    // If we also have a left mapping then we need to build a copy of the subset of the protection
    // ranges for this right mapping. We do this first here as this might need to allocate and can
    // fail.
    arch_mmu_flags_t right_first_mmu_flags = 0;
    btree::BTree<vaddr_t, arch_mmu_flags_t> right_prot_ranges;
    const vaddr_t offset = base + size - this->base();
    if (left) {
      zx_status_t status = CopyProtectionRangesLocked(base + size, this->size() - offset,
                                                      &right_first_mmu_flags, &right_prot_ranges);
      if (status != ZX_OK) {
        return status;
      }
    }
    fbl::RefPtr<VmObject> vmo = object_locked();
    right = fbl::ImportFromRawPtr(rust_vm_mapping_create_with_ranges(
        parent_locked(), private_clone(), this->base() + offset, this->size() - offset, flags(),
        fbl::ExportToRawPtr(&vmo), object_offset() + offset, right_first_mmu_flags,
        &right_prot_ranges, Mergeable::YES));
    if (!right) {
      return ZX_ERR_NO_MEMORY;
    }
  }

  // Grab the lock for the vmo. This is acquired here so that it is held continuously over both the
  // architectural unmap and removing the current mapping from the VMO.
  DEBUG_ASSERT(object_locked());
  Guard<CriticalMutex> guard{object_locked()->lock()};

  // With the object allocations done the last action that could fail is insertion into the
  // subregions_ list and object mapping list. Notionally we want to remove the old mapping, then
  // install our new mapping(s), but if our installation fails then we cannot necessarily rollback
  // and install the old mapping. To work around this we instead first install any right mapping,
  // which could fail and then if there is a left mapping replace the current mapping, which cannot
  // fail. This approach has the result of temporarily causing the subregions_ list to have an
  // overlapping entry and for the parent to have unactivated mappings, however we hold both the
  // main and subregion lock over the entire operation, and so this is never visible. Similarly we
  // hold the object lock over the entire manipulation of mappings, and so the mappings being added
  // before being in the alive state is never visible.
  if (right) {
    zx_status_t status = parent_locked()->subregions_.InsertRegion(
        fbl::RefPtr<VmAddressRegionOrMapping>(right.get()));
    if (status != ZX_OK) {
      return status;
    }
    status = object_locked()->AddMappingLocked(right.get());
    if (status != ZX_OK) {
      parent_locked()->subregions_.RemoveRegion(right.get());
      return status;
    }
  }
  if (left) {
    zx_status_t status = object_locked()->AddMappingLocked(left.get());
    if (status != ZX_OK) {
      if (right) {
        object_locked()->RemoveMappingLocked(right.get());
        parent_locked()->subregions_.RemoveRegion(right.get());
      }
      return status;
    }
    // Replace can never fail as it does not need to allocate.
    parent_locked()->subregions_.ReplaceRegion(this,
                                               fbl::RefPtr<VmAddressRegionOrMapping>(left.get()));
  } else {
    parent_locked()->subregions_.RemoveRegion(this);
  }

  zx_status_t status =
      aspace()->arch_aspace().Unmap(base, size / kPageSize, aspace()->EnlargeArchUnmap());
  ASSERT(status == ZX_OK);

  const MemoryPriority old_priority = memory_priority_locked();
  auto set_priority =
      [old_priority](VmMapping& self) TA_REQ(self.lock()) TA_REQ(self.object_locked()->lock()) {
        if (old_priority == VmAddressRegion::MemoryPriority::HIGH) {
          self.SetMemoryPriorityHighAlreadyPositiveLockedObject</*SplitOnUnmap=*/true>();
        } else {
          DEBUG_ASSERT(old_priority == VmAddressRegion::MemoryPriority::DEFAULT);
          self.SetMemoryPriorityDefaultLockedObject</*SplitOnUnmap=*/true>();
        }
      };

  // Split the protection_ranges_ from this mapping into the new mapping(s). This has be done after
  // the mapping construction as this step is destructive and hard to rollback.
  //
  // Need to set memory priorities before we call DestroyLockedObject. If we have
  // MemoryPriority::HIGH, then we need to pass that on to left and right before the object and
  // the aspace suffer any dynamic reclamation.
  if (right) {
    AssertHeld(right->lock_ref());
    AssertHeld(right->object_lock_ref());
    // If there was a left mapping we already populated a new protection_ranges above when we
    // created the right mapping, for the other case we need to erase the unneeded parts of our
    // protection_ranges_ and move it over.
    if (!left) {
      right->set_first_region_arch_mmu_flags_locked(RemoveAfterSplitLocked(base + size));
      right->rest_protection_ranges_ = ktl::move(rest_protection_ranges_);
    }
    set_priority(*right);
  }
  if (left) {
    AssertHeld(left->lock_ref());
    AssertHeld(left->object_lock_ref());
    // Erase the top part of protection_ranges_ and then move it to the left mapping.
    ClearProtectionRangeTransitionsLocked(base, this->base() + this->size());
    left->set_first_region_arch_mmu_flags_locked(first_region_arch_mmu_flags_locked());
    left->rest_protection_ranges_ = ktl::move(rest_protection_ranges_);
    set_priority(*left);
  }

  // Now finish destroying this mapping. We already updated the subregion_ list in the parent.
  status = DestroyLockedObject(DestroyUnmap::No, DestroyRemoveFromParent::No);
  ASSERT(status == ZX_OK);

  // Install the new mappings.
  auto finish_mapping = [](fbl::RefPtr<VmMapping>& mapping) {
    if (mapping) {
      AssertHeld(mapping->lock_ref());
      AssertHeld(mapping->object_lock_ref());
      AssertHeld(mapping->region_lock_ref());
      // We already updated the parent region list and object mapping list.
      mapping->ActivateNoInsertLocked();
    }
  };
  finish_mapping(left);
  finish_mapping(right);
  return ZX_OK;
}

zx_status_t VmMapping::DecommitRange(size_t offset, size_t len) {
  return rust_vm_mapping_decommit_range(this, offset, len);
}

zx_status_t VmMapping::DestroyLockedObject(DestroyUnmap unmap,
                                           DestroyRemoveFromParent remove_region) {
  return rust_vm_mapping_destroy_locked_object(this, unmap, remove_region);
}

void VmMapping::ActivateNoInsertLocked() { rust_vm_mapping_activate_no_insert_locked(this); }

fbl::RefPtr<VmMapping> VmMapping::TryMergeRightNeighborLocked(VmMapping* right_candidate)
    TA_NO_THREAD_SAFETY_ANALYSIS {
  // Take a reference to ourself, so that we do not get destructed if we drop the last reference to
  // ourself due to erasing from our parent.
  fbl::RefPtr<VmMapping> self(this);

  AssertHeld(right_candidate->lock_ref());
  AssertHeld(right_candidate->region_lock_ref());

  // This code is tolerant of many 'miss calls' if mappings aren't mergeable or are not neighbours
  // etc, but the caller should not be attempting to merge if these mappings are not actually from
  // the same vmar parent. Doing so indicates something structurally wrong with the hierarchy.
  DEBUG_ASSERT(parent_locked() == right_candidate->parent_locked());

  // Should not be able to have the same parent yet have gotten a different memory priority.
  DEBUG_ASSERT(memory_priority_locked() == right_candidate->memory_priority_locked());

  // These tests are intended to be ordered such that we fail as fast as possible. As such testing
  // for mergeability, which we commonly expect to succeed and not fail, is done last.

  // Need to refer to the same object.
  if (object_locked().get() != right_candidate->object_locked().get()) {
    return nullptr;
  }
  DEBUG_ASSERT(private_clone() == right_candidate->private_clone());
  // Aspace and VMO ranges need to be contiguous. Validate that the right candidate is actually to
  // the right in addition to checking that base+size lines up for single scenario where
  // base()+size() can overflow and becomes zero.
  if (base() + size() != right_candidate->base() || right_candidate->base() < base()) {
    return nullptr;
  }
  if (object_offset() + size() != right_candidate->object_offset()) {
    return nullptr;
  }
  // All flags need to be consistent.
  if (flags() != right_candidate->flags()) {
    return nullptr;
  }
  // Although we can combine the protect_region_list_rest_ of the two mappings, we require that they
  // be of the same cacheability, as this is an assumption that mapping has a single cacheability
  // type. Since all protection regions have the same cacheability we can check any arbitrary one in
  // each of the mappings. Note that this check is technically redundant, since a VMO can only have
  // one kind of cacheability and we already know this is the same VMO, but some extra paranoia here
  // does not hurt.
  if ((first_region_arch_mmu_flags_locked() & ARCH_MMU_FLAG_CACHE_MASK) !=
      (right_candidate->first_region_arch_mmu_flags_locked() & ARCH_MMU_FLAG_CACHE_MASK)) {
    return nullptr;
  }

  // Only merge live mappings.
  if (state_locked() != LifeCycleState::ALIVE ||
      right_candidate->state_locked() != LifeCycleState::ALIVE) {
    return nullptr;
  }
  // Both need to be mergeable.
  if (mergeable_locked() == Mergeable::NO || right_candidate->mergeable_locked() == Mergeable::NO) {
    return nullptr;
  }

  fbl::RefPtr<VmObject> vmo = object_locked();
  btree::BTree<vaddr_t, arch_mmu_flags_t> prot_ranges;
  fbl::RefPtr<VmMapping> new_mapping = fbl::ImportFromRawPtr(rust_vm_mapping_create_with_ranges(
      parent_locked(), private_clone(), base(), size() + right_candidate->size(), flags(),
      fbl::ExportToRawPtr(&vmo), object_offset(), 0, &prot_ranges, Mergeable::YES));
  if (!new_mapping) {
    return nullptr;
  }
  AssertHeld(new_mapping->lock_ref());

  const MemoryPriority old_priority = memory_priority_locked();
  // Although it is somewhat awkward and verbose, we use a lambda here instead of just a subscope to
  // prevent the usages of `AssertHeld` from 'leaking' beyond the actual guard scope.
  const bool failure = [&]() TA_REQ(lock()) TA_REQ(right_candidate->lock()) TA_REQ(
                           new_mapping->lock()) {
    // Although it was safe to read size() without holding the object lock, we need to acquire it
    // to perform changes.
    Guard<CriticalMutex> guard{AliasedLock, object_locked()->lock(),
                               right_candidate->object_locked()->lock()};

    AssertHeld(new_mapping->object_lock_ref());
    zx_status_t status = new_mapping->object_locked()->AddMappingLocked(new_mapping.get());
    if (status != ZX_OK) {
      return true;
    }

    // Attempt to copy all the protection ranges first as this might fail due to an allocation.
    // If it fails we can still abort with a fairly minor roll-back procedure.
    if (MergeProtectionRangesLocked(*right_candidate) != ZX_OK) {
      new_mapping->object_locked()->RemoveMappingLocked(new_mapping.get());
      // On failure roll back any protection ranges that might have been inserted. Erase cannot
      // fail.
      ClearProtectionRangeTransitionsLocked(base() + size(),
                                            base() + size() + right_candidate->size());
      return true;
    }

    AssertHeld(region_lock_ref());

    new_mapping->set_first_region_arch_mmu_flags_locked(first_region_arch_mmu_flags_locked());
    new_mapping->rest_protection_ranges_ = ktl::move(rest_protection_ranges_);

    AssertHeld(right_candidate->region_lock_ref());
    // First destroy the right hand mapping, and remove it from the parent.
    status = right_candidate->DestroyLockedObject(DestroyUnmap::No, DestroyRemoveFromParent::Yes);
    ASSERT(status == ZX_OK);
    AssertHeld(parent_locked()->lock_ref());
    AssertHeld(parent_locked()->region_lock_ref());
    // To avoid activate_locked from failing we use ReplaceRegion to swap the left mapping for
    // the new mapping in the subregions_ list. This temporarily results in the subregions_
    // list having overlapping mappings and an unactivated mapping, but as we hold both the main
    // lock and subregion lock over the entire operation this state cannot be observed.
    parent_locked()->subregions_.ReplaceRegion(
        this, fbl::RefPtr<VmAddressRegionOrMapping>(new_mapping.get()));
    status = DestroyLockedObject(DestroyUnmap::No, DestroyRemoveFromParent::No);
    ASSERT(status == ZX_OK);
    AssertHeld(new_mapping->region_lock_ref());
    new_mapping->ActivateNoInsertLocked();
    return false;
  }();
  if (failure) {
    return nullptr;
  }

  new_mapping->SetMemoryPriorityLocked(old_priority);

  vm_mappings_merged.Add(1);
  return new_mapping;
}

void VmMapping::TryMergeNeighborsLocked() {
  AssertCanary();

  // Check that this mapping is mergeable and is currently in the correct lifecycle state.
  if (mergeable_locked() == Mergeable::NO || state_locked() != LifeCycleState::ALIVE) {
    return;
  }
  // As a VmMapping if we we are alive we by definition have a parent.
  DEBUG_ASSERT(parent_locked());

  // We expect there to be a RefPtr to us held beyond the one for the wavl tree ensuring that we
  // cannot trigger our own destructor should we remove ourselves from the hierarchy.
  DEBUG_ASSERT(ref_count_debug() > 1);

  AssertHeld(parent_locked()->lock_ref());
  AssertHeld(parent_locked()->region_lock_ref());

  // Find our two merge candidates.
  fbl::RefPtr<VmMapping> left, right;
  if (auto left_candidate = parent_locked()->subregions_.LeftOf(this); left_candidate.IsValid()) {
    left = (*left_candidate).second->as_vm_mapping();
  }
  if (auto right_candidate = parent_locked()->subregions_.RightOf(this);
      right_candidate.IsValid()) {
    right = (*right_candidate).second->as_vm_mapping();
  }

  // Attempt to merge with each candidate. Any successful merge will produce a new mapping and
  // invalidate this.
  if (right) {
    right = TryMergeRightNeighborLocked(right.get());
  }
  if (left) {
    // We either merge the left with our result of the right merge, or if that was not successful
    // with |this|.
    AssertHeld(left->lock_ref());
    AssertHeld(left->region_lock_ref());
    left->TryMergeRightNeighborLocked(right ? right.get() : this);
  }
}

void VmMapping::MarkMergeable(fbl::RefPtr<VmMapping> mapping) {
  Guard<CriticalMutex> region_guard{mapping->region_lock()};
  Guard<CriticalMutex> guard{mapping->lock()};
  // Now that we have the lock check this mapping is still alive and we haven't raced with some
  // kind of destruction.
  if (mapping->state_locked() != LifeCycleState::ALIVE) {
    return;
  }
  // Skip marking any vdso segments mergeable. Although there is currently only one vdso segment and
  // so it would never actually get merged, marking it mergeable is technically incorrect.
  if (mapping->aspace()->vdso_code_mapping_ == mapping) {
    return;
  }
  mapping->set_mergeable_locked(Mergeable::YES);
  mapping->TryMergeNeighborsLocked();
}

template <bool SplitOnUnmap>
void VmMapping::SetMemoryPriorityLocked(VmAddressRegion::MemoryPriority priority) {
  if constexpr (SplitOnUnmap) {
    rust_vm_mapping_set_mem_priority_locked_split(this, priority);
  } else {
    rust_vm_mapping_set_mem_priority_locked(this, priority);
  }
}
template void VmMapping::SetMemoryPriorityLocked<false>(VmAddressRegion::MemoryPriority);
template void VmMapping::SetMemoryPriorityLocked<true>(VmAddressRegion::MemoryPriority);

template <bool SplitOnUnmap>
void VmMapping::SetMemoryPriorityDefaultLockedObject() {
  if constexpr (SplitOnUnmap) {
    rust_vm_mapping_set_mem_priority_default_locked_object_split(this);
  } else {
    rust_vm_mapping_set_mem_priority_default_locked_object(this);
  }
}
template void VmMapping::SetMemoryPriorityDefaultLockedObject<false>();
template void VmMapping::SetMemoryPriorityDefaultLockedObject<true>();

template <bool SplitOnUnmap>
void VmMapping::SetMemoryPriorityHighAlreadyPositiveLockedObject() {
  if constexpr (SplitOnUnmap) {
    rust_vm_mapping_set_mem_priority_high_already_positive_locked_object_split(this);
  } else {
    rust_vm_mapping_set_mem_priority_high_already_positive_locked_object(this);
  }
}
template void VmMapping::SetMemoryPriorityHighAlreadyPositiveLockedObject<false>();
template void VmMapping::SetMemoryPriorityHighAlreadyPositiveLockedObject<true>();

zx::result<fbl::RefPtr<VmMapping>> VmMapping::ForceWritable() {
  AssertCanary();
  // Take a ref to ourselves in case we drop the last one when removing from our parent.
  fbl::RefPtr<VmMapping> self(this);
  Guard<CriticalMutex> region_guard{region_lock()};
  Guard<CriticalMutex> guard{lock()};
  if (state_locked() != LifeCycleState::ALIVE) {
    return zx::error{ZX_ERR_BAD_STATE};
  }
  DEBUG_ASSERT(object_locked());
  DEBUG_ASSERT(parent_locked());

  // Never allow writes to the vdso.
  if (aspace()->vdso_code_mapping_.get() == this) {
    return zx::error(ZX_ERR_ACCESS_DENIED);
  }
  // If we have already re-directed to a private clone then there is no need to do so again.
  if (private_clone()) {
    return zx::ok(ktl::move(self));
  }
  // If the mapping is already possible to write to (even if disabled by current protections), then
  // writing is already safe.
  if (is_valid_mapping_flags(ARCH_MMU_FLAG_PERM_WRITE)) {
    return zx::ok(ktl::move(self));
  }
  // A physical VMO cannot be cloned and so we cannot make this safe, just allow the write.
  if (!object_locked()->is_paged()) {
    return zx::ok(ktl::move(self));
  }

  // Create a clone of our VMO that covers the size of our mapping.
  fbl::RefPtr<VmMapping> writable;
  {
    fbl::RefPtr<VmObject> clone;
    zx_status_t status = object_locked()->CreateClone(
        Resizability::NonResizable, SnapshotType::OnWrite, object_offset(), size(), true, &clone);
    if (status != ZX_OK) {
      return zx::error(status);
    }
    if (flags() & VMAR_FLAG_FAULT_BEYOND_STREAM_SIZE) {
      VmObjectPaged* paged = DownCastVmObject<VmObjectPaged>(object_locked().get());
      DEBUG_ASSERT(paged);
      uint64_t original_stream_size;
      {
        Guard<CriticalMutex> object_guard{AliasedLock, paged->lock(), object_lock()};
        ktl::optional<uint64_t> stream_size = paged->user_stream_size_locked();
        DEBUG_ASSERT(stream_size.has_value());
        original_stream_size = *stream_size;
      }
      const uint64_t stream_size_over_offset =
          ktl::max(original_stream_size, object_offset()) - object_offset();
      const uint64_t stream_size_limited_by_map_size = ktl::min(stream_size_over_offset, size());
      zx::result<fbl::RefPtr<StreamSizeManager>> result =
          StreamSizeManager::Create(stream_size_limited_by_map_size);
      if (result.is_error()) {
        return zx::error(result.error_value());
      }
      clone->SetUserStreamSize(ktl::move(*result));
    }
    // TODO(https://fxbug.dev/503042881) Support a more efficient deep copy.
    btree::BTree<vaddr_t, arch_mmu_flags_t> protection_ranges;
    for (auto [key, value] : rest_protection_ranges_locked()) {
      auto result = protection_ranges.insert(key, value);
      if (!result) {
        return zx::error(ZX_ERR_NO_MEMORY);
      }
    }

    // We created the clone starting at our object offset in the old object, so that makes the
    // equivalent start offset be 0 in the clone.
    writable = fbl::ImportFromRawPtr(rust_vm_mapping_create_with_ranges(
        parent_locked(), true, base(), size(), flags(), fbl::ExportToRawPtr(&clone), 0,
        first_region_arch_mmu_flags_locked(), &protection_ranges, mergeable_locked()));
    if (!writable) {
      return zx::error(ZX_ERR_NO_MEMORY);
    }
    // First transfer any memory priority from the current mapping to the new mapping.
    AssertHeld(writable->lock_ref());
    AssertHeld(writable->region_lock_ref());
    // Use SplitOnUnmap=true because writable hasn't been activated yet.
    writable->SetMemoryPriorityLocked</*SplitOnUnmap=*/true>(memory_priority_locked());

    Guard<CriticalMutex> object_guard{writable->object_lock()};
    status = writable->object_locked()->AddMappingLocked(writable.get());
    if (status != ZX_OK) {
      writable->SetMemoryPriorityDefaultLockedObject</*SplitOnUnmap=*/true>();
      return zx::error(status);
    }

    AssertHeld(parent_locked()->lock_ref());
    AssertHeld(parent_locked()->region_lock_ref());
    parent_locked()->subregions_.ReplaceRegion(
        this, fbl::RefPtr<VmAddressRegionOrMapping>(writable.get()));
    writable->ActivateNoInsertLocked();
  }
  // Now acquire the original object lock and destroy ourself.
  {
    // Keep a refptr to the object so we know our lock remains valid.
    fbl::RefPtr<VmObject> object(object_locked());
    Guard<CriticalMutex> object_guard{object_lock()};
    zx_status_t status = DestroyLockedObject(DestroyUnmap::Yes, DestroyRemoveFromParent::No);
    ASSERT(status == ZX_OK);
  }
  return zx::ok(ktl::move(writable));
}
