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

// Helpers called from Rust
zx_status_t cpp_restricted_state_create_vmo_mapping(void** out_vmo, void** out_mapping,
                                                    zx_restricted_state_t** out_base);
void cpp_restricted_state_destroy_vmo_mapping(void* raw_vmo, void* raw_mapping);
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
