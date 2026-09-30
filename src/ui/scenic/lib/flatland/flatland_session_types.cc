// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "src/ui/scenic/lib/flatland/flatland_session_types.h"

#include <lib/fit/result.h>
#include <lib/zx/event.h>
#include <lib/zx/handle.h>
#include <zircon/errors.h>
#include <zircon/status.h>
#include <zircon/syscalls/object.h>
#include <zircon/types.h>

#include <utility>
#include <variant>

namespace flatland {

fit::result<zx_status_t, WaitFence> WaitFence::From(fuchsia_ui_composition::wire::WaitFence fence) {
  WaitFence result;
  switch (fence.Which()) {
    case fuchsia_ui_composition::wire::WaitFence::Tag::kBasic: {
      zx_handle_t raw_handle = fence.basic().get();
      if (raw_handle == ZX_HANDLE_INVALID) {
        return fit::error(ZX_ERR_BAD_HANDLE);
      }
      zx_info_handle_basic_t info;
      zx_status_t status = zx_object_get_info(raw_handle, ZX_INFO_HANDLE_BASIC, &info, sizeof(info),
                                              nullptr, nullptr);
      if (status != ZX_OK) {
        return fit::error(status);
      }
      if (info.type != ZX_OBJ_TYPE_EVENT && info.type != ZX_OBJ_TYPE_EVENTPAIR) {
        return fit::error(ZX_ERR_WRONG_TYPE);
      }
      if (!(info.rights & ZX_RIGHT_WAIT)) {
        return fit::error(ZX_ERR_ACCESS_DENIED);
      }
      zx::handle h = std::move(fence.basic());
      result.fence_ = zx::event(h.release());
      return fit::ok(std::move(result));
    }
    case fuchsia_ui_composition::wire::WaitFence::Tag::kTimestamp: {
      zx_handle_t raw_handle = fence.timestamp().get();
      if (raw_handle == ZX_HANDLE_INVALID) {
        return fit::error(ZX_ERR_BAD_HANDLE);
      }
      zx_info_handle_basic_t info;
      zx_status_t status = zx_object_get_info(raw_handle, ZX_INFO_HANDLE_BASIC, &info, sizeof(info),
                                              nullptr, nullptr);
      if (status != ZX_OK) {
        return fit::error(status);
      }
      if (info.type != ZX_OBJ_TYPE_COUNTER) {
        return fit::error(ZX_ERR_WRONG_TYPE);
      }
      if (!(info.rights & ZX_RIGHT_WAIT)) {
        return fit::error(ZX_ERR_ACCESS_DENIED);
      }
      result.fence_ = std::move(fence.timestamp());
      return fit::ok(std::move(result));
    }
  }
}

bool WaitFence::is_valid() const {
  if (std::holds_alternative<zx::event>(fence_)) {
    return std::get<zx::event>(fence_).is_valid();
  }
  if (std::holds_alternative<zx::counter>(fence_)) {
    return std::get<zx::counter>(fence_).is_valid();
  }
  return false;
}

zx::handle WaitFence::TakeHandle() {
  if (std::holds_alternative<zx::event>(fence_)) {
    zx::event ev = std::move(std::get<zx::event>(fence_));
    fence_ = std::monostate{};
    return ev;
  }
  if (std::holds_alternative<zx::counter>(fence_)) {
    zx::counter c = std::move(std::get<zx::counter>(fence_));
    fence_ = std::monostate{};
    return c;
  }
  return zx::handle{};
}

}  // namespace flatland
