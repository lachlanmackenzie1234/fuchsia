// Copyright 2018 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include "vm/pmm_node.h"

#include <assert.h>
#include <inttypes.h>
#include <lib/instrumentation/asan.h>
#include <lib/memalloc/range.h>
#include <lib/page/size.h>

#include <kernel/scheduler.h>
#include <phys/handoff/handoff.h>
#include <vm/compression.h>
#include <vm/phys/arena.h>
#include <vm/physmap.h>
#include <vm/pmm.h>
#include <vm/pmm_node_ffi.h>
#include <vm/vm_aspace.h>

// We disable thread safety analysis here, since this function is only called
// during early boot before threading exists.
zx_status_t PmmNode::Init(ktl::span<const memalloc::Range> ranges) TA_NO_THREAD_SAFETY_ANALYSIS {
  return rust_pmm_node_init(this, ranges.data(), ranges.size());
}

void PmmNode::EndHandoff() { rust_pmm_node_end_handoff(this); }

zx_status_t PmmNode::GetArenaInfo(size_t count, uint64_t i, pmm_arena_info_t* buffer,
                                  size_t buffer_size) {
  return rust_pmm_node_get_arena_info(this, count, i, buffer, buffer_size);
}

void PmmNode::FillFreePagesAndArm() { rust_pmm_node_fill_free_pages_and_arm(this); }

void PmmNode::CheckAllFreePages() { rust_pmm_node_check_all_free_pages(this); }

#if __has_feature(address_sanitizer)
void PmmNode::PoisonAllFreePages() { rust_pmm_node_poison_all_free_pages(this); }
#endif  // __has_feature(address_sanitizer)

bool PmmNode::EnableFreePageFilling(size_t fill_size, CheckFailAction action) {
  return rust_pmm_node_enable_free_page_filling(this, fill_size, static_cast<uint8_t>(action));
}

zx::result<vm_page_t*> PmmNode::AllocLoanedPage(
    fit::inline_function<void(vm_page_t*), 32> allocated) {
  vm_page_t* out_page = nullptr;
  auto cb = [](vm_page_t* page, void* cookie) {
    auto* fn = reinterpret_cast<fit::inline_function<void(vm_page_t*), 32>*>(cookie);
    (*fn)(page);
  };
  zx_status_t status = rust_pmm_node_alloc_loaned_page(this, cb, &allocated, &out_page);
  if (status != ZX_OK) {
    return zx::error(status);
  }
  return zx::ok(out_page);
}

zx::result<vm_page_t*> PmmNode::AllocPage(uint alloc_flags) {
  vm_page_t* page = nullptr;
  zx_status_t status = rust_pmm_node_alloc_page(this, alloc_flags, &page);
  if (status != ZX_OK) {
    return zx::error(status);
  }
  return zx::ok(page);
}

zx_status_t PmmNode::AllocPages(size_t count, uint alloc_flags, VmPageDoublyLinkedList* list) {
  return rust_pmm_node_alloc_pages(this, count, alloc_flags, list);
}

zx_status_t PmmNode::AllocRange(paddr_t address, size_t count, VmPageDoublyLinkedList* list) {
  return rust_pmm_node_alloc_range(this, address, count, list);
}

zx_status_t PmmNode::AllocContiguous(const size_t count, uint alloc_flags, uint8_t alignment_log2,
                                     paddr_t* pa, VmPageDoublyLinkedList* list) {
  return rust_pmm_node_alloc_contiguous(this, count, alloc_flags, alignment_log2, pa, list);
  // Forbid zero size contiguous allocations because we are obligated to provide the physical
  // address on success, but have no sensible value to give.
}

void PmmNode::BeginFreeLoanedPage(vm_page_t* page,
                                  fit::inline_function<void(vm_page_t*)> release_page,
                                  FreeLoanedPagesHolder& flph) {
  auto cb = [](vm_page_t* page, void* cookie) {
    auto* fn = reinterpret_cast<fit::inline_function<void(vm_page_t*)>*>(cookie);
    (*fn)(page);
  };
  rust_pmm_node_begin_free_loaned_page(this, page, cb, &release_page, &flph);
}

void PmmNode::FinishFreeLoanedPages(FreeLoanedPagesHolder& flph) {
  rust_pmm_node_finish_free_loaned_pages(this, &flph);
}

void PmmNode::FreePage(vm_page* page, PmmOptDelayReuse delay_reuse) {
  rust_pmm_node_free_page(this, page, delay_reuse);
}

void PmmNode::BeginFreeLoanedArray(
    vm_page_t** pages, size_t count,
    fit::inline_function<void(vm_page_t**, size_t, VmPageDoublyLinkedList*)> release_list,
    FreeLoanedPagesHolder& flph) {
  auto cb = [](vm_page_t** pages, size_t count, VmPageDoublyLinkedList* list, void* cookie) {
    auto* fn =
        reinterpret_cast<fit::inline_function<void(vm_page_t**, size_t, VmPageDoublyLinkedList*)>*>(
            cookie);
    (*fn)(pages, count, list);
  };
  rust_pmm_node_begin_free_loaned_array(this, pages, count, cb, &release_list, &flph);
}

void PmmNode::FreeList(VmPageDoublyLinkedList* list, PmmOptDelayReuse delay_reuse) {
  rust_pmm_node_free_list(this, list, delay_reuse);
}

void PmmNode::UnwirePage(vm_page* page) { rust_pmm_node_unwire_page(this, page); }

uint64_t PmmNode::CountFreePages() const TA_NO_THREAD_SAFETY_ANALYSIS {
  return rust_pmm_node_count_free_pages(this);
}

uint64_t PmmNode::CountLoanedFreePages() const TA_NO_THREAD_SAFETY_ANALYSIS {
  return rust_pmm_node_count_loaned_free_pages(this);
}

uint64_t PmmNode::CountLoanedNotFreePages() const TA_NO_THREAD_SAFETY_ANALYSIS {
  return rust_pmm_node_count_loaned_not_free_pages(this);
}

uint64_t PmmNode::CountLoanedPages() const TA_NO_THREAD_SAFETY_ANALYSIS {
  return rust_pmm_node_count_loaned_pages(this);
}

uint64_t PmmNode::CountLoanCancelledPages() const TA_NO_THREAD_SAFETY_ANALYSIS {
  return rust_pmm_node_count_loan_cancelled_pages(this);
}

uint64_t PmmNode::CountTotalBytes() const TA_NO_THREAD_SAFETY_ANALYSIS {
  return rust_pmm_node_count_total_bytes(this);
}

void PmmNode::DumpFree() const TA_NO_THREAD_SAFETY_ANALYSIS { rust_pmm_node_dump_free(this); }

void PmmNode::Dump(bool is_panic) const { rust_pmm_node_dump(this, is_panic); }

void PmmNode::ReportAllocFailure(AllocFailure failure) {
  rust_pmm_node_report_alloc_failure(this, &failure);
}

void PmmNode::SeedRandomShouldWait() { rust_pmm_node_seed_random_should_wait(this); }

zx::result<vm_page_t*> PmmNode::WaitForSinglePageAllocation(Deadline deadline, bool suspendable) {
  vm_page_t* out_page = nullptr;
  zx_status_t status = rust_pmm_node_wait_for_single_page_allocation(
      this, deadline.when(), deadline.slack().amount(), deadline.slack().mode(), suspendable,
      &out_page);
  if (status != ZX_OK) {
    return zx::error(status);
  }
  return zx::ok(out_page);
}
