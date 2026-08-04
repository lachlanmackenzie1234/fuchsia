// Copyright 2019 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "usb/request-fidl.h"

#include <lib/fake-bti/bti.h>
#include <lib/fit/defer.h>

#include <zxtest/zxtest.h>

namespace {

TEST(RequestFidlTest, EmptyRequestTest) {
  fuchsia_hardware_usb_request::Request request;
  usb::FidlRequest fidl_request(std::move(request));
}

TEST(RequestFidlTest, LengthTest) {
  usb::FidlRequest fidl_request;
  fidl_request.add_vmo_id(0, 16, 0).add_data({}, 16, 0);

  EXPECT_EQ(fidl_request.length(), 32);
}

TEST(RequestFidlTest, UnpinTest) {
  zx::vmo vmo;
  ASSERT_OK(zx::vmo::create(16, 0, &vmo));
  fuchsia_hardware_usb_request::Request request;
  request.data()
      .emplace()
      .emplace_back()
      .buffer(fuchsia_hardware_usb_request::Buffer::WithVmoId(1))
      .offset(0)
      .size(32);
  request.data()
      ->emplace_back()
      .buffer(
          fuchsia_hardware_usb_request::Buffer::WithData(std::vector<uint8_t>{0x0, 0x0, 0x0, 0x0}))
      .offset(0)
      .size(4);
  usb::FidlRequest fidl_request(std::move(request));

  zx::bti fake_bti;
  ASSERT_OK(fake_bti_create(fake_bti.reset_and_get_address()));

  EXPECT_OK(fidl_request.PhysMap(fake_bti));
  size_t actual;
  fake_bti_pinned_vmo_info_t info[1];
  EXPECT_OK(fake_bti_get_pinned_vmos(fake_bti.get(), info, 1, &actual));
  EXPECT_EQ(actual, 1);

  void* mapped;
  EXPECT_OK(zx::vmar::root_self()->map(ZX_VM_PERM_READ | ZX_VM_PERM_WRITE, 0, zx::vmo(info[0].vmo),
                                       0, info[0].size, reinterpret_cast<uintptr_t*>(&mapped)));
  auto iter3 = fidl_request.phys_iter(1, zx_system_get_page_size());
  EXPECT_EQ((*iter3.begin()).second, 4);
  uint8_t expected_vals[] = {0xA, 0xB, 0xC, 0xC};
  memcpy(mapped, expected_vals, sizeof(expected_vals));
  EXPECT_OK(zx::vmar::root_self()->unmap(reinterpret_cast<uintptr_t>(mapped), info[0].size));

  EXPECT_OK(fidl_request.Unpin());
  EXPECT_OK(fake_bti_get_pinned_vmos(fake_bti.get(), nullptr, 0, &actual));
  EXPECT_EQ(actual, 0);
  EXPECT_BYTES_EQ((*fidl_request->data())[1].buffer()->data()->data(), expected_vals,
                  sizeof(expected_vals));
}

TEST(RequestFidlTest, VmoIdTest) {
  fuchsia_hardware_usb_request::Request request;
  request.data()
      .emplace()
      .emplace_back()
      .buffer(fuchsia_hardware_usb_request::Buffer::WithVmoId(0))
      .offset(0)
      .size(16);
  request.data()
      ->emplace_back()
      .buffer(fuchsia_hardware_usb_request::Buffer::WithVmoId(1))
      .offset(0)
      .size(16);
  request.data()
      .emplace()
      .emplace_back()
      .buffer(fuchsia_hardware_usb_request::Buffer::WithVmoId(2))
      .offset(0)
      .size(16);
  usb::FidlRequest fidl_request(std::move(request));

  zx::bti fake_bti;
  ASSERT_OK(fake_bti_create(fake_bti.reset_and_get_address()));

  EXPECT_OK(fidl_request.PhysMap(fake_bti));
  size_t actual;
  EXPECT_OK(fake_bti_get_pinned_vmos(fake_bti.get(), nullptr, 0, &actual));
  EXPECT_EQ(actual, 0);
}

TEST(RequestFidlTest, DataTest) {
  fuchsia_hardware_usb_request::Request request;
  uint8_t expected1[] = {0xF, 0xE, 0xD, 0xC, 0xB, 0xA, 0x9, 0x8,
                         0x7, 0x6, 0x5, 0x4, 0x3, 0x2, 0x1, 0x0};
  request.data()
      .emplace()
      .emplace_back()
      .buffer(fuchsia_hardware_usb_request::Buffer::WithData(
          std::vector<uint8_t>(std::begin(expected1), std::end(expected1))))
      .offset(0)
      .size(16);
  uint8_t expected2[] = {0x0, 0x1, 0x2, 0x3, 0x4, 0x5, 0x6, 0x7, 0x8, 0x9, 0xA,
                         0xB, 0xC, 0xD, 0xE, 0xF, 0x0, 0x1, 0x2, 0x3, 0x4, 0x5,
                         0x6, 0x7, 0x8, 0x9, 0xA, 0xB, 0xC, 0xD, 0xE, 0xF};
  request.data()
      ->emplace_back()
      .buffer(fuchsia_hardware_usb_request::Buffer::WithData(
          std::vector<uint8_t>(std::begin(expected2), std::end(expected2))))
      .offset(0)
      .size(32);
  usb::FidlRequest fidl_request(std::move(request));

  zx::bti fake_bti;
  ASSERT_OK(fake_bti_create(fake_bti.reset_and_get_address()));

  EXPECT_OK(fidl_request.PhysMap(fake_bti));
  size_t actual;
  fake_bti_pinned_vmo_info_t info[2];
  EXPECT_OK(fake_bti_get_pinned_vmos(fake_bti.get(), info, 2, &actual));
  EXPECT_EQ(actual, 2);

  void* mapped;
  EXPECT_OK(zx::vmar::root_self()->map(ZX_VM_PERM_READ | ZX_VM_PERM_WRITE, 0, zx::vmo(info[0].vmo),
                                       0, info[0].size, reinterpret_cast<uintptr_t*>(&mapped)));
  auto iter1 = fidl_request.phys_iter(0, zx_system_get_page_size());
  EXPECT_BYTES_EQ(mapped, expected1, sizeof(expected1));
  EXPECT_EQ((*iter1.begin()).second, 16);
  EXPECT_OK(zx::vmar::root_self()->unmap(reinterpret_cast<uintptr_t>(mapped), info[0].size));

  EXPECT_OK(zx::vmar::root_self()->map(ZX_VM_PERM_READ | ZX_VM_PERM_WRITE, 0, zx::vmo(info[1].vmo),
                                       0, info[1].size, reinterpret_cast<uintptr_t*>(&mapped)));
  auto iter2 = fidl_request.phys_iter(1, zx_system_get_page_size());
  EXPECT_BYTES_EQ(mapped, expected2, sizeof(expected2));
  EXPECT_EQ((*iter2.begin()).second, 32);
  EXPECT_OK(zx::vmar::root_self()->unmap(reinterpret_cast<uintptr_t>(mapped), info[1].size));
}

TEST(RequestFidlTest, MixedTest) {
  std::vector<uint8_t> tmp(32);
  usb::FidlRequest fidl_request;
  fidl_request.add_vmo_id(3, 16, 0).add_vmo_id(7, 16, 0).add_data(std::move(tmp), 32, 0);

  zx::bti fake_bti;
  ASSERT_OK(fake_bti_create(fake_bti.reset_and_get_address()));

  EXPECT_OK(fidl_request.PhysMap(fake_bti));
  size_t actual;
  EXPECT_OK(fake_bti_get_pinned_vmos(fake_bti.get(), nullptr, 0, &actual));
  EXPECT_EQ(actual, 1);

  auto iter = fidl_request.phys_iter(2, zx_system_get_page_size());
  EXPECT_EQ((*iter.begin()).second, 32);
}

TEST(RequestFidlTest, PoolTest) {
  usb::FidlRequestPool pool;
  EXPECT_TRUE(pool.Empty());

  pool.Add(usb::FidlRequest(usb::EndpointType::BULK));
  EXPECT_TRUE(pool.Full());
  usb::FidlRequest control(usb::EndpointType::CONTROL);
  control.add_vmo_id(9, 2, 0).add_vmo_id(1, 4, 0);
  pool.Add(std::move(control));
  EXPECT_TRUE(pool.Full());

  {
    auto req = pool.Get();
    EXPECT_TRUE(req.has_value());
    EXPECT_FALSE(pool.Empty());
    EXPECT_FALSE(pool.Full());
    EXPECT_EQ(req->request().information()->Which(),
              fuchsia_hardware_usb_request::RequestInfo::Tag::kBulk);

    pool.Put(std::move(*req));
    EXPECT_TRUE(pool.Full());
  }

  {
    auto req = pool.Get();
    EXPECT_TRUE(req.has_value());
    EXPECT_FALSE(pool.Empty());
    EXPECT_FALSE(pool.Full());
    EXPECT_EQ(req->request().information()->Which(),
              fuchsia_hardware_usb_request::RequestInfo::Tag::kControl);
    EXPECT_EQ(req->request().data()->size(), 2);
    EXPECT_EQ(req->request().data()->at(0).buffer()->vmo_id().value(), 9);
    EXPECT_EQ(req->request().data()->at(0).size(), 2);
    EXPECT_EQ(req->request().data()->at(0).offset(), 0);
    EXPECT_EQ(req->request().data()->at(1).buffer()->vmo_id().value(), 1);
    EXPECT_EQ(req->request().data()->at(1).size(), 4);
    EXPECT_EQ(req->request().data()->at(1).offset(), 0);
  }

  {
    auto req = pool.Remove();
    EXPECT_TRUE(req.has_value());
    EXPECT_TRUE(pool.Empty());

    pool.Put(std::move(*req));
    EXPECT_TRUE(pool.Full());
  }

  {
    auto req = pool.Remove();
    EXPECT_TRUE(req.has_value());
    EXPECT_TRUE(pool.Empty());
  }

  {
    auto req = pool.Remove();
    EXPECT_FALSE(req.has_value());
  }

  // Make sure that we are able to destruct even with requests sitting in the pool.
  pool.Add(usb::FidlRequest(usb::EndpointType::BULK));
  EXPECT_TRUE(pool.Full());
}

TEST(RequestFidlTest, CopyAndCacheTest) {
  zx::vmo vmo;
  ASSERT_OK(zx::vmo::create(zx_system_get_page_size(), 0, &vmo));
  uintptr_t mapped_addr = 0;
  ASSERT_OK(zx::vmar::root_self()->map(ZX_VM_PERM_READ | ZX_VM_PERM_WRITE, 0, vmo, 0,
                                       zx_system_get_page_size(), &mapped_addr));
  auto unmap_guard = fit::defer([mapped_addr]() {
    EXPECT_OK(zx::vmar::root_self()->unmap(mapped_addr, zx_system_get_page_size()));
  });
  auto get_mapped = [mapped_addr](const fuchsia_hardware_usb_request::Buffer& buffer)
      -> zx::result<std::optional<usb::internal::MappedVmo>> {
    return zx::ok(usb::internal::MappedVmo{mapped_addr, zx_system_get_page_size()});
  };

  usb::FidlRequest fidl_request;
  fidl_request.add_vmo_id(0, 16, 0);

  uint8_t write_data[32] = {1,  2,  3,  4,  5,  6,  7,  8,  9,  10, 11, 12, 13, 14, 15, 16,
                            17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32};
  auto cp_res = fidl_request.CachedCopyTo(0, write_data, 16, get_mapped);
  ASSERT_OK(cp_res.status_value());
  ASSERT_EQ(cp_res.value().size(), 1u);
  EXPECT_EQ(cp_res.value()[0], 16u);

  uint8_t read_data[32] = {};
  cp_res = fidl_request.CachedCopyFrom(0, read_data, 16, get_mapped);
  ASSERT_OK(cp_res.status_value());
  ASSERT_EQ(cp_res.value().size(), 1u);
  EXPECT_EQ(cp_res.value()[0], 16u);
  EXPECT_BYTES_EQ(read_data, write_data, 16);

  // Zero-length copies.
  cp_res = fidl_request.CachedCopyTo(0, write_data, 0, get_mapped);
  ASSERT_OK(cp_res.status_value());
  ASSERT_EQ(cp_res.value().size(), 1u);
  EXPECT_EQ(cp_res.value()[0], 0u);
  cp_res = fidl_request.CachedCopyFrom(0, read_data, 0, get_mapped);
  ASSERT_OK(cp_res.status_value());
  ASSERT_EQ(cp_res.value().size(), 1u);
  EXPECT_EQ(cp_res.value()[0], 0u);

  // Out-of-bounds offset test.
  cp_res = fidl_request.CachedCopyTo(100, write_data, 16, get_mapped);
  ASSERT_STATUS(cp_res.status_value(), ZX_ERR_OUT_OF_RANGE);
  cp_res = fidl_request.CachedCopyFrom(100, read_data, 16, get_mapped);
  ASSERT_STATUS(cp_res.status_value(), ZX_ERR_OUT_OF_RANGE);

  // Scatter-Gather multi-region test.
  usb::FidlRequest sg_request;
  sg_request.add_vmo_id(0, 16, 0).add_vmo_id(0, 16, 16);
  cp_res = sg_request.CachedCopyTo(0, write_data, 32, get_mapped);
  ASSERT_OK(cp_res.status_value());
  ASSERT_EQ(cp_res.value().size(), 2u);
  EXPECT_EQ(cp_res.value()[0], 16u);
  EXPECT_EQ(cp_res.value()[1], 16u);

  memset(read_data, 0, sizeof(read_data));
  cp_res = sg_request.CachedCopyFrom(0, read_data, 32, get_mapped);
  ASSERT_OK(cp_res.status_value());
  ASSERT_EQ(cp_res.value().size(), 2u);
  EXPECT_EQ(cp_res.value()[0], 16u);
  EXPECT_EQ(cp_res.value()[1], 16u);
  EXPECT_BYTES_EQ(read_data, write_data, 32);

  // Inline data buffers test.
  auto get_mapped_inline = [](const fuchsia_hardware_usb_request::Buffer& buffer)
      -> zx::result<std::optional<usb::internal::MappedVmo>> { return zx::ok(std::nullopt); };
  usb::FidlRequest inline_request;
  inline_request.add_data({}, 16, 0).add_data({}, 16, 16);
  cp_res = inline_request.CachedCopyTo(0, write_data, 32, get_mapped_inline);
  ASSERT_OK(cp_res.status_value());
  ASSERT_EQ(cp_res.value().size(), 2u);
  EXPECT_EQ(cp_res.value()[0], 16u);
  EXPECT_EQ(cp_res.value()[1], 16u);

  memset(read_data, 0, sizeof(read_data));
  cp_res = inline_request.CachedCopyFrom(0, read_data, 32, get_mapped_inline);
  ASSERT_OK(cp_res.status_value());
  ASSERT_EQ(cp_res.value().size(), 2u);
  EXPECT_EQ(cp_res.value()[0], 16u);
  EXPECT_EQ(cp_res.value()[1], 16u);
  EXPECT_BYTES_EQ(read_data, write_data, 32);

  // Inline data offset skipping test.
  cp_res = inline_request.CachedCopyTo(16, write_data, 16, get_mapped_inline);
  ASSERT_OK(cp_res.status_value());
  ASSERT_EQ(cp_res.value().size(), 2u);
  EXPECT_EQ(cp_res.value()[0], 0u);
  EXPECT_EQ(cp_res.value()[1], 16u);

  // Mapping error propagation test.
  auto bad_get_mapped = [](const fuchsia_hardware_usb_request::Buffer& buffer)
      -> zx::result<std::optional<usb::internal::MappedVmo>> {
    return zx::error(ZX_ERR_BAD_HANDLE);
  };
  cp_res = fidl_request.CachedCopyTo(0, write_data, 16, bad_get_mapped);
  ASSERT_STATUS(cp_res.status_value(), ZX_ERR_BAD_HANDLE);
  cp_res = fidl_request.CachedCopyFrom(0, read_data, 16, bad_get_mapped);
  ASSERT_STATUS(cp_res.status_value(), ZX_ERR_BAD_HANDLE);
}

TEST(RequestFidlTest, RangeCacheFlushTest) {
  zx::vmo vmo;
  ASSERT_OK(zx::vmo::create(zx_system_get_page_size(), 0, &vmo));
  uintptr_t mapped_addr = 0;
  ASSERT_OK(zx::vmar::root_self()->map(ZX_VM_PERM_READ | ZX_VM_PERM_WRITE, 0, vmo, 0,
                                       zx_system_get_page_size(), &mapped_addr));
  auto unmap_guard = fit::defer([mapped_addr]() {
    EXPECT_OK(zx::vmar::root_self()->unmap(mapped_addr, zx_system_get_page_size()));
  });
  auto get_mapped = [mapped_addr](const fuchsia_hardware_usb_request::Buffer& buffer)
      -> zx::result<std::optional<usb::internal::MappedVmo>> {
    return zx::ok(usb::internal::MappedVmo{mapped_addr, zx_system_get_page_size()});
  };

  // 1. Flushing a partial slice of a VMO buffer.
  usb::FidlRequest fidl_request;
  fidl_request.add_vmo_id(0, 64, 0);
  EXPECT_OK(fidl_request.CacheFlush(get_mapped, 16, 32));
  EXPECT_OK(fidl_request.CacheFlushInvalidate(get_mapped, 16, 32));

  // 2. Selective get_mapped to verify untouched preceding and subsequent regions are skipped.
  usb::FidlRequest sg_request;
  sg_request.add_vmo_id(0, 16, 0).add_vmo_id(1, 16, 16).add_vmo_id(2, 16, 32);
  auto get_mapped_selective = [mapped_addr](const fuchsia_hardware_usb_request::Buffer& buffer)
      -> zx::result<std::optional<usb::internal::MappedVmo>> {
    if (buffer.vmo_id().value() == 1) {
      return zx::ok(usb::internal::MappedVmo{mapped_addr, zx_system_get_page_size()});
    }
    return zx::error(ZX_ERR_BAD_STATE);
  };
  // Only flushing buffer 1 (offset 16 to 32) should succeed without querying buffer 0 or 2.
  EXPECT_OK(sg_request.CacheFlush(get_mapped_selective, 16, 16));
  EXPECT_OK(sg_request.CacheFlushInvalidate(get_mapped_selective, 16, 16));
  // Flushing buffer 0 or 2 should fail.
  EXPECT_STATUS(sg_request.CacheFlush(get_mapped_selective, 0, 16), ZX_ERR_BAD_STATE);
  EXPECT_STATUS(sg_request.CacheFlushInvalidate(get_mapped_selective, 0, 16), ZX_ERR_BAD_STATE);
  EXPECT_STATUS(sg_request.CacheFlush(get_mapped_selective, 32, 16), ZX_ERR_BAD_STATE);
  EXPECT_STATUS(sg_request.CacheFlushInvalidate(get_mapped_selective, 32, 16), ZX_ERR_BAD_STATE);

  // 3. Flushing across multiple scatter-gather VMO regions.
  EXPECT_OK(sg_request.CacheFlush(get_mapped, 8, 32));
  EXPECT_OK(sg_request.CacheFlushInvalidate(get_mapped, 8, 32));

  // 4. Zero-length and out-of-bounds bounds.
  EXPECT_OK(fidl_request.CacheFlush(get_mapped, 0, 0));
  EXPECT_OK(fidl_request.CacheFlushInvalidate(get_mapped, 0, 0));
  EXPECT_OK(fidl_request.CacheFlush(get_mapped, 10, 0));
  EXPECT_OK(fidl_request.CacheFlushInvalidate(get_mapped, 10, 0));
  // Out of bounds offset.
  EXPECT_STATUS(fidl_request.CacheFlush(get_mapped, 1000, 16), ZX_ERR_OUT_OF_RANGE);
  EXPECT_STATUS(fidl_request.CacheFlushInvalidate(get_mapped, 1000, 16), ZX_ERR_OUT_OF_RANGE);
  // Partial out of bounds size (starts in bounds, exceeds total length).
  EXPECT_OK(fidl_request.CacheFlush(get_mapped, 48, 100));
  EXPECT_OK(fidl_request.CacheFlushInvalidate(get_mapped, 48, 100));

  // 5. Zero-length buffer region and inline data region skipping.
  usb::FidlRequest mixed_request;
  mixed_request.add_vmo_id(0, 16, 0)
      .add_vmo_id(0, 0, 16)
      .add_data({}, 16, 16)
      .add_vmo_id(0, 16, 32);
  auto get_mapped_mixed = [mapped_addr](const fuchsia_hardware_usb_request::Buffer& buffer)
      -> zx::result<std::optional<usb::internal::MappedVmo>> {
    if (buffer.Which() == fuchsia_hardware_usb_request::Buffer::Tag::kData) {
      return zx::ok(std::nullopt);
    }
    return zx::ok(usb::internal::MappedVmo{mapped_addr, zx_system_get_page_size()});
  };
  EXPECT_OK(mixed_request.CacheFlush(get_mapped_mixed, 0, 48));
  EXPECT_OK(mixed_request.CacheFlushInvalidate(get_mapped_mixed, 0, 48));
}

}  // namespace
