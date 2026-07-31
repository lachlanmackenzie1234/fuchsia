// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include <lib/fit/defer.h>
#include <zircon/types.h>

#include <fbl/ref_ptr.h>
#include <kernel/restricted_state.h>
#include <vm/vm_address_region.h>
#include <vm/vm_aspace.h>
#include <vm/vm_object.h>
#include <vm/vm_object_paged.h>

extern "C" {

zx_status_t rust_restricted_state_create(zx_exception_report_t* exception_report_ptr,
                                         RestrictedState** out_ptr);
void rust_restricted_state_destroy(RestrictedState* ptr);

void rust_arch_dump(const zx_restricted_state_t* state);
zx_status_t rust_arch_validate_state_pre_restricted_entry(const zx_restricted_state_t* state);
void rust_arch_save_state_pre_restricted_entry(ArchSavedNormalState* state);
[[noreturn]] void rust_arch_enter_restricted(const zx_restricted_state_t* state);
void rust_arch_save_restricted_syscall_state(zx_restricted_state_t* state,
                                             const syscall_regs_t* regs);
void rust_arch_save_restricted_iframe_state(zx_restricted_state_t* state, const iframe_t* frame);
void rust_arch_save_restricted_exception_state(zx_restricted_state_t* state);
void rust_arch_redirect_restricted_exception_to_normal(const ArchSavedNormalState* arch_state,
                                                       uintptr_t vector_table, uintptr_t context,
                                                       zx_restricted_reason_t reason);
[[noreturn]] void rust_arch_enter_full(const ArchSavedNormalState* arch_state,
                                       uintptr_t vector_table, uintptr_t context, uint64_t code);

// Helpers called from Rust
zx_status_t cpp_restricted_state_create_vmo_mapping(void** out_vmo, void** out_mapping,
                                                    zx_restricted_state_t** out_base);
void cpp_restricted_state_destroy_vmo_mapping(void* raw_vmo, void* raw_mapping);

#if defined(__aarch64__)
bool cpp_arm64_ints_disabled();
void cpp_arm64_get_tpidr_regs(uint64_t* tpidr_el0, uint64_t* tpidrro_el0);
void cpp_arm64_set_tpidr_regs(uint64_t tpidr_el0, uint64_t tpidrro_el0);
void cpp_arm64_enter_restricted_tpidr(uint64_t tpidr_el0, bool is_arm32);
uint64_t cpp_arm64_get_tpidr_el0();
[[noreturn]] void cpp_arm64_enter_uspace(const iframe_t* iframe);
zx_status_t cpp_arm64_get_general_regs(zx_thread_state_general_regs_t* regs);
zx_status_t cpp_arm64_set_general_regs(const zx_thread_state_general_regs_t* regs);
#endif  // defined(__aarch64__)
}

// This function:
//   1. Creates a 1-page paged VMO (`state_vmo`) to back the thread's restricted mode state.
//   2. Commits and pins the VMO (`CommitRangePinned`) so physical pages are permanently backed.
//   3. Creates a mapping of this VMO in the kernel address space (`kernel_aspace`).
//   4. Eagerly faults in all pages (`MapRange`) so kernel mode never demand-faults on access.
zx_status_t cpp_restricted_state_create_vmo_mapping(void** out_vmo, void** out_mapping,
                                                    zx_restricted_state_t** out_base) {
  static constexpr size_t kStateVmoSize = kPageSize;
  static constexpr uint32_t kVmoOptions = 0;
  static constexpr uint32_t kPmmAllocFlags = PMM_ALLOC_FLAG_ANY | PMM_ALLOC_FLAG_CAN_WAIT;
  fbl::RefPtr<VmObjectPaged> state_vmo;
  zx_status_t status =
      VmObjectPaged::Create(kPmmAllocFlags, kVmoOptions, kStateVmoSize, &state_vmo);
  if (status != ZX_OK) {
    return status;
  }

  status = state_vmo->CommitRangePinned(0, kStateVmoSize, /*write=*/true);
  if (status != ZX_OK) {
    return status;
  }

  fbl::RefPtr<VmAddressRegion> kernel_vmar =
      VmAspace::kernel_aspace()->RootVmar()->as_vm_address_region();
  zx::result<VmAddressRegion::MapResult> state_mapping_result = kernel_vmar->CreateVmMapping(
      0, kStateVmoSize, 0, 0, state_vmo, 0, ARCH_MMU_FLAG_PERM_READ | ARCH_MMU_FLAG_PERM_WRITE,
      "restricted state");
  if (state_mapping_result.is_error()) {
    state_vmo->Unpin(0, kStateVmoSize);
    return state_mapping_result.error_value();
  }

  status = state_mapping_result->mapping->MapRange(0, kStateVmoSize, true);
  if (status != ZX_OK) {
    state_mapping_result->mapping->Destroy();
    state_vmo->Unpin(0, kStateVmoSize);
    return status;
  }

  *out_vmo = fbl::ExportToRawPtr(&state_vmo);
  *out_mapping = fbl::ExportToRawPtr(&state_mapping_result->mapping);
  *out_base = reinterpret_cast<zx_restricted_state_t*>(state_mapping_result->base);
  return ZX_OK;
}

void cpp_restricted_state_destroy_vmo_mapping(void* raw_vmo, void* raw_mapping) {
  if (raw_mapping) {
    auto mapping = fbl::ImportFromRawPtr<VmMapping>(static_cast<VmMapping*>(raw_mapping));
    mapping->Destroy();
  }
  if (raw_vmo) {
    static constexpr size_t kStateVmoSize = kPageSize;
    auto vmo = fbl::ImportFromRawPtr<VmObjectPaged>(static_cast<VmObjectPaged*>(raw_vmo));
    vmo->Unpin(0, kStateVmoSize);
  }
}

zx::result<ktl::unique_ptr<RestrictedState>> RestrictedState::Create(
    user_out_ptr<zx_exception_report_t> exception_report_ptr) {
  RestrictedState* ptr = nullptr;
  zx_status_t status = rust_restricted_state_create(exception_report_ptr.get(), &ptr);
  if (status != ZX_OK) {
    return zx::error_result(status);
  }
  return zx::ok(ktl::unique_ptr<RestrictedState>(ptr));
}

RestrictedState::~RestrictedState() { rust_restricted_state_destroy(this); }

fbl::RefPtr<VmObjectPaged> RestrictedState::vmo() const {
  if (!vmo_) {
    return fbl::RefPtr<VmObjectPaged>();
  }
  return fbl::RefPtr<VmObjectPaged>(reinterpret_cast<VmObjectPaged*>(vmo_));
}

void RestrictedState::ArchDump(const zx_restricted_state_t& state) { rust_arch_dump(&state); }

zx_status_t RestrictedState::ArchValidateStatePreRestrictedEntry(
    const zx_restricted_state_t& state) {
  return rust_arch_validate_state_pre_restricted_entry(&state);
}

void RestrictedState::ArchSaveStatePreRestrictedEntry(ArchSavedNormalState& arch_state) {
  rust_arch_save_state_pre_restricted_entry(&arch_state);
}

[[noreturn]] void RestrictedState::ArchEnterRestricted(const zx_restricted_state_t& state) {
  rust_arch_enter_restricted(&state);
}

void RestrictedState::ArchSaveRestrictedSyscallState(zx_restricted_state_t& state,
                                                     const syscall_regs_t& regs) {
  rust_arch_save_restricted_syscall_state(&state, &regs);
}

void RestrictedState::ArchSaveRestrictedIframeState(zx_restricted_state_t& state,
                                                    const iframe_t& frame) {
  rust_arch_save_restricted_iframe_state(&state, &frame);
}

void RestrictedState::ArchSaveRestrictedExceptionState(zx_restricted_state_t& state) {
  rust_arch_save_restricted_exception_state(&state);
}

void RestrictedState::ArchRedirectRestrictedExceptionToNormal(
    const ArchSavedNormalState& arch_state, uintptr_t vector_table, uintptr_t context,
    zx_restricted_reason_t reason) {
  rust_arch_redirect_restricted_exception_to_normal(&arch_state, vector_table, context, reason);
}

[[noreturn]] void RestrictedState::ArchEnterFull(const ArchSavedNormalState& arch_state,
                                                 uintptr_t vector_table, uintptr_t context,
                                                 uint64_t code) {
  rust_arch_enter_full(&arch_state, vector_table, context, code);
}
