// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::handle::HandleValue;

unsafe extern "C" {
    fn cpp_resource_validate_resource_kind_base(
        handle: zx_types::zx_handle_t,
        kind: zx_types::zx_rsrc_kind_t,
        base: zx_types::zx_rsrc_system_base_t,
    ) -> zx_types::zx_status_t;
    fn cpp_resource_validate_ranged_resource(
        handle: zx_types::zx_handle_t,
        kind: zx_types::zx_rsrc_kind_t,
        base: usize,
        size: usize,
    ) -> zx_types::zx_status_t;
}

pub fn validate_resource_kind_base(
    handle: HandleValue,
    kind: zx_types::zx_rsrc_kind_t,
    base: zx_types::zx_rsrc_system_base_t,
) -> Result<(), zx_status::Status> {
    // SAFETY: The FFI function is safe to call with any handle value and kind/base options.
    zx_status::Status::ok(unsafe {
        cpp_resource_validate_resource_kind_base(handle.raw_value(), kind, base)
    })
}

pub fn validate_ranged_resource(
    handle: HandleValue,
    kind: zx_types::zx_rsrc_kind_t,
    base: usize,
    size: usize,
) -> Result<(), zx_status::Status> {
    // SAFETY: The FFI function is safe to call with any handle value and kind/base options.
    zx_status::Status::ok(unsafe {
        cpp_resource_validate_ranged_resource(handle.raw_value(), kind, base, size)
    })
}
