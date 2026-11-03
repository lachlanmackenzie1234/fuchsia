// Copyright 2019 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include <fake_codec_adapter.h>
#include <fuchsia/media/cpp/fidl.h>
#include <fuchsia/media/drm/cpp/fidl.h>
#include <fuchsia/mediacodec/cpp/fidl.h>
#include <lib/async-loop/cpp/loop.h>
#include <lib/async-loop/default.h>
#include <lib/media/codec_impl/fourcc.h>
#include <lib/zx/time.h>

#include <gtest/gtest.h>

#include "lib/media/codec_impl/codec_impl.h"
#include "src/lib/testing/loop_fixture/real_loop_fixture.h"

namespace {

auto CreateDecoderParams() {
  fuchsia::mediacodec::CreateDecoder_Params params;
  params.mutable_input_details()->set_format_details_version_ordinal(0).set_mime_type("video/vp9");
  return params;
}

auto CreateEncoderParams() {
  fuchsia::mediacodec::CreateEncoder_Params params;
  params.mutable_input_details()->set_format_details_version_ordinal(0).set_mime_type("audio/sbc");
  return params;
}

auto CreateDecryptorParams() {
  fuchsia::media::drm::DecryptorParams params;
  params.mutable_input_details()->set_format_details_version_ordinal(0);
  return params;
}

}  // namespace

class CodecImplLifetime : public gtest::RealLoopFixture {
 protected:
  CodecImplLifetime()
      : loop_separate_thread_(&kAsyncLoopConfigNoAttachToCurrentThread),
        admission_control_(dispatcher()) {
    // nothing else to do here
  }

  ~CodecImplLifetime() {
    // Force to teardown before admission_control_.
    RunLoopUntilIdle();
    QuitLoop();
  }

  void SetUp() override { loop_separate_thread_.StartThread("separate_thread"); }
  void TearDown() override {
    // Force any failure during ~CodecImpl to have more obvious stack.
    codec_impl_ = nullptr;
  }

  void Create(bool bind = true, bool delete_async = false,
              CodecImpl::StreamProcessorParams params = CreateDecoderParams()) {
    // Just hold onto the server end and never connect it to anything, for now.
    auto sysmem_endpoints = fidl::CreateEndpoints<fuchsia_sysmem2::Allocator>();
    ZX_ASSERT(sysmem_endpoints.is_ok());
    sysmem_client_ = std::move(sysmem_endpoints->client);
    sysmem_request_ = std::move(sysmem_endpoints->server);
    codec_request_ = codec_client_handle_.NewRequest();
    ZX_DEBUG_ASSERT(!delete_async || bind);
    std::unique_ptr<CodecImpl> codec_impl;
    admission_control_.TryAddCodec(
        true, [this, bind, delete_async, params = std::move(params),
               &codec_impl](std::unique_ptr<CodecAdmission> codec_admission) mutable {
          codec_impl = std::make_unique<CodecImpl>(
              std::move(sysmem_client_), std::move(codec_admission), dispatcher(), thrd_current(),
              std::move(params), std::move(codec_request_));
          auto fake_codec_adapter =
              std::make_unique<FakeCodecAdapter>(codec_impl->lock(), codec_impl.get());
          fake_codec_adapter_ = fake_codec_adapter.get();
          codec_impl->SetCoreCodecAdapter(std::move(fake_codec_adapter));
          if (!bind) {
            return;
          }
          codec_impl->BindAsync([this, delete_async] {
            if (!delete_async) {
              codec_impl_ = nullptr;
            } else {
              zx_status_t status = async::PostTask(dispatcher(), [this] { codec_impl_ = nullptr; });
              ZX_DEBUG_ASSERT(status == ZX_OK);
            }
            error_handler_ran_ = true;
          });
        });
    RunLoopUntil([&codec_impl] { return !!codec_impl; });
    ZX_DEBUG_ASSERT(codec_impl);
    codec_impl_ = std::move(codec_impl);
  }

  void StartSyncChain() {
    if (codec_client_ptr_) {
      codec_client_ptr_->Sync([this] {
        ++sync_completion_count_;
        StartSyncChain();
      });
    }
  }

  void PostToSeparateThread(fit::closure to_run) {
    zx_status_t status = async::PostTask(loop_separate_thread_.dispatcher(), std::move(to_run));
    ZX_DEBUG_ASSERT(status == ZX_OK);
  }

  async::Loop loop_separate_thread_;
  CodecAdmissionControl admission_control_;

  // The server end isn't connected to anything, for now.
  fidl::ClientEnd<fuchsia_sysmem2::Allocator> sysmem_client_;
  fidl::ServerEnd<fuchsia_sysmem2::Allocator> sysmem_request_;

  // Tests that don't intend to process any received messages use
  // InterfaceHandle while those that do intend to process received messages
  // use InterfacePtr.
  fidl::InterfaceHandle<fuchsia::media::StreamProcessor> codec_client_handle_;
  fuchsia::media::StreamProcessorPtr codec_client_ptr_;

  fidl::InterfaceRequest<fuchsia::media::StreamProcessor> codec_request_;

  std::unique_ptr<CodecImpl> codec_impl_;
  FakeCodecAdapter* fake_codec_adapter_;
  bool error_handler_ran_ = false;
  uint64_t sync_completion_count_ = 0;
};

TEST_F(CodecImplLifetime, CreateDelete) {
  Create(false);
  codec_impl_ = nullptr;
  RunLoopUntilIdle();
  EXPECT_FALSE(error_handler_ran_);
}

TEST_F(CodecImplLifetime, CreateBindDelete) {
  // This exercises BindAsync vs. ~CodecImpl, so run several times.
  for (uint32_t i = 0; i < 200; ++i) {
    Create();
    codec_impl_ = nullptr;
    RunLoopUntilIdle();
    EXPECT_FALSE(error_handler_ran_);
  }
}

TEST_F(CodecImplLifetime, CreateBindChannelClose) {
  Create();
  // Close the client end of the StreamProcessor channel.
  codec_client_handle_.TakeChannel().reset();
  RunLoopUntil([this] { return error_handler_ran_; });
  EXPECT_TRUE(error_handler_ran_);
}

TEST_F(CodecImplLifetime, CreateBindChannelCloseDeleteAsync) {
  Create(true, true);
  // Close the client end of the StreamProcessor channel.
  codec_client_handle_.TakeChannel().reset();
  RunLoopUntil([this] { return error_handler_ran_; });
  // This doesn't imply that !codec_impl_ yet, because deletion
  // is happening async.
  EXPECT_TRUE(error_handler_ran_);
  RunLoopUntil([this] { return !codec_impl_; });
  EXPECT_TRUE(!codec_impl_);
}

TEST_F(CodecImplLifetime, CreateBindChannelCloseDeleteAsyncWithOngoingSyncs) {
  // More than one thread is involved, so do this several times in case it helps catch something
  // bad that doesn't always happen.
  constexpr uint64_t kIterCount = 20;
  for (uint64_t iter = 0; iter < kIterCount; ++iter) {
    Create(true, true);

    codec_client_ptr_.Bind(codec_client_handle_.TakeChannel());
    constexpr uint32_t kInFlightSyncTarget = 5;
    for (uint32_t i = 0; i < kInFlightSyncTarget; ++i) {
      // Each started chain kicks another Sync() on each completion, any time
      // RunLoopUntil() is running.
      StartSyncChain();
    }

    // Make sure the sync chains will re-trigger new syncs while RunLoopUntil() is
    // running.
    RunLoopUntil([this] { return sync_completion_count_ >= kInFlightSyncTarget * 2; });

    // Trigger an error as if the FakeCodecAdapter had triggered it, with a slight
    // delay before the trigger so we get some coverage of syncs happening
    // continuously while the failure handling happens.
    PostToSeparateThread([this] {
      zx::nanosleep(zx::deadline_after(zx::msec(20)));
      static_cast<CodecAdapterEvents*>(codec_impl_.get())
          ->onCoreCodecFailCodec(
              "CreateBindChannelCloseDeleteAsyncWithOngoingSyncs triggering failure");
    });

    RunLoopUntil([this] { return !codec_impl_; });
    EXPECT_TRUE(!codec_impl_);
  }
}

TEST_F(CodecImplLifetime, CreateBindDeleteEncoder) {
  // This exercises BindAsync vs. ~CodecImpl, so run several times.
  for (uint32_t i = 0; i < 200; ++i) {
    Create(true, false, CreateEncoderParams());
    codec_impl_ = nullptr;
    RunLoopUntilIdle();
    EXPECT_FALSE(error_handler_ran_);
  }
}

TEST_F(CodecImplLifetime, CreateBindDeleteDecryptor) {
  // This exercises BindAsync vs. ~CodecImpl, so run several times.
  for (uint32_t i = 0; i < 200; ++i) {
    Create(true, false, CreateDecryptorParams());
    codec_impl_ = nullptr;
    RunLoopUntilIdle();
    EXPECT_FALSE(error_handler_ran_);
  }
}

TEST(CodecImplFormatDetailsBridge, VideoV1ToV2) {
  fuchsia::media::FormatDetails format_details;
  format_details.set_format_details_version_ordinal(1);
  format_details.set_mime_type("video/raw");

  fuchsia::media::VideoUncompressedFormat uncompressed;
  uncompressed.image_format.pixel_format.type = fuchsia::sysmem::PixelFormatType::NV12;
  uncompressed.image_format.coded_width = 1920;
  uncompressed.image_format.coded_height = 1088;
  uncompressed.image_format.bytes_per_row = 1920;
  uncompressed.image_format.display_width = 1920;
  uncompressed.image_format.display_height = 1080;
  uncompressed.image_format.layers = 1;
  uncompressed.image_format.color_space.type = fuchsia::sysmem::ColorSpaceType::REC709;
  uncompressed.image_format.has_pixel_aspect_ratio = true;
  uncompressed.image_format.pixel_aspect_ratio_width = 4;
  uncompressed.image_format.pixel_aspect_ratio_height = 3;
  format_details.mutable_domain()->video().set_uncompressed(std::move(uncompressed));

  CodecImpl::EnsureFormatDetailsBridged(&format_details);

  ASSERT_TRUE(format_details.has_image_format());
  const auto& v2 = format_details.image_format();
  ASSERT_TRUE(v2.has_pixel_format());
  EXPECT_EQ(v2.pixel_format(), fuchsia::images2::PixelFormat::NV12);
  ASSERT_TRUE(v2.has_size());
  EXPECT_EQ(v2.size().width, 1920u);
  EXPECT_EQ(v2.size().height, 1088u);
  ASSERT_TRUE(v2.has_bytes_per_row());
  EXPECT_EQ(v2.bytes_per_row(), 1920u);
  ASSERT_TRUE(v2.has_display_rect());
  EXPECT_EQ(v2.display_rect().x, 0u);
  EXPECT_EQ(v2.display_rect().y, 0u);
  EXPECT_EQ(v2.display_rect().width, 1920u);
  EXPECT_EQ(v2.display_rect().height, 1080u);
  ASSERT_TRUE(v2.has_color_space());
  EXPECT_EQ(v2.color_space(), fuchsia::images2::ColorSpace::REC709);
  ASSERT_TRUE(v2.has_pixel_aspect_ratio());
  EXPECT_EQ(v2.pixel_aspect_ratio().width, 4u);
  EXPECT_EQ(v2.pixel_aspect_ratio().height, 3u);

  // Bridging V1 -> V2 also normalizes `domain.video.uncompressed` and is strictly idempotent.
  EXPECT_EQ(format_details.domain().video().uncompressed().fourcc, make_fourcc('N', 'V', '1', '2'));
  EXPECT_TRUE(format_details.domain().video().uncompressed().planar);
  EXPECT_EQ(format_details.domain().video().uncompressed().primary_width_pixels, 1920u);
  EXPECT_EQ(format_details.domain().video().uncompressed().primary_height_pixels, 1088u);
  fuchsia::media::FormatDetails bridged_once = fidl::Clone(format_details);
  CodecImpl::EnsureFormatDetailsBridged(&format_details);
  EXPECT_TRUE(fidl::Equals(bridged_once, format_details));

  // When V1 omits both display_width/height and primary_display_width/height_pixels,
  // `display_rect` must be unset rather than `{0, 0, 0, 0}`.
  {
    fuchsia::media::FormatDetails no_display_details;
    no_display_details.set_format_details_version_ordinal(2);
    auto& no_disp_v1 = no_display_details.mutable_domain()->video().uncompressed();
    no_disp_v1.image_format.pixel_format.type = fuchsia::sysmem::PixelFormatType::NV12;
    no_disp_v1.image_format.coded_width = 1280;
    no_disp_v1.image_format.coded_height = 720;
    no_disp_v1.image_format.bytes_per_row = 1280;
    no_disp_v1.image_format.color_space.type = fuchsia::sysmem::ColorSpaceType::REC709;

    CodecImpl::EnsureFormatDetailsBridged(&no_display_details);
    ASSERT_TRUE(no_display_details.has_image_format());
    EXPECT_FALSE(no_display_details.image_format().has_display_rect());
  }

  // Bridging when `pixel_format` is omitted (`INVALID`) in V1 -> V2 and V2 -> V1.
  {
    fuchsia::media::FormatDetails v1_no_pf;
    v1_no_pf.set_format_details_version_ordinal(3);
    auto& uncompressed_no_pf = v1_no_pf.mutable_domain()->video().uncompressed();
    uncompressed_no_pf.image_format.coded_width = 640;
    uncompressed_no_pf.image_format.coded_height = 480;
    uncompressed_no_pf.image_format.display_width = 640;
    uncompressed_no_pf.image_format.display_height = 480;
    uncompressed_no_pf.image_format.color_space.type = fuchsia::sysmem::ColorSpaceType::REC709;

    CodecImpl::EnsureFormatDetailsBridged(&v1_no_pf);
    ASSERT_TRUE(v1_no_pf.has_image_format());
    EXPECT_FALSE(v1_no_pf.image_format().has_pixel_format());
    ASSERT_TRUE(v1_no_pf.image_format().has_size());
    EXPECT_EQ(v1_no_pf.image_format().size().width, 640u);
    EXPECT_EQ(v1_no_pf.image_format().size().height, 480u);
    ASSERT_TRUE(v1_no_pf.image_format().has_display_rect());
    EXPECT_EQ(v1_no_pf.image_format().display_rect().width, 640u);
    EXPECT_EQ(v1_no_pf.image_format().display_rect().height, 480u);

    fuchsia::media::FormatDetails v2_no_pf;
    v2_no_pf.set_format_details_version_ordinal(4);
    v2_no_pf.mutable_image_format()->set_size({.width = 640, .height = 480});
    v2_no_pf.mutable_image_format()->set_display_rect(
        {.x = 0, .y = 0, .width = 640, .height = 480});

    CodecImpl::EnsureFormatDetailsBridged(&v2_no_pf);
    ASSERT_TRUE(v2_no_pf.has_domain());
    ASSERT_TRUE(v2_no_pf.domain().is_video());
    ASSERT_TRUE(v2_no_pf.domain().video().is_uncompressed());
    EXPECT_EQ(v2_no_pf.domain().video().uncompressed().image_format.pixel_format.type,
              fuchsia::sysmem::PixelFormatType::INVALID);
    EXPECT_EQ(v2_no_pf.domain().video().uncompressed().image_format.coded_width, 640u);
    EXPECT_EQ(v2_no_pf.domain().video().uncompressed().image_format.coded_height, 480u);
  }
}

TEST(CodecImplFormatDetailsBridge, VideoV2ToV1Nv12AndYv12) {
  {
    fuchsia::media::FormatDetails format_details;
    format_details.set_format_details_version_ordinal(1);
    format_details.set_mime_type("video/raw");

    auto* v2 = format_details.mutable_image_format();
    v2->set_pixel_format(fuchsia::images2::PixelFormat::NV12);
    v2->set_size({.width = 1280, .height = 736});
    v2->set_bytes_per_row(1280);
    v2->set_display_rect({.x = 0, .y = 0, .width = 1280, .height = 720});
    v2->set_color_space(fuchsia::images2::ColorSpace::REC709);
    v2->set_pixel_aspect_ratio({.width = 16, .height = 9});

    CodecImpl::EnsureFormatDetailsBridged(&format_details);

    ASSERT_TRUE(format_details.has_domain());
    ASSERT_TRUE(format_details.domain().is_video());
    ASSERT_TRUE(format_details.domain().video().is_uncompressed());
    const auto& v1 = format_details.domain().video().uncompressed();
    EXPECT_EQ(v1.fourcc, make_fourcc('N', 'V', '1', '2'));
    EXPECT_TRUE(v1.planar);
    EXPECT_FALSE(v1.swizzled);
    EXPECT_EQ(v1.primary_width_pixels, 1280u);
    EXPECT_EQ(v1.primary_height_pixels, 736u);
    EXPECT_EQ(v1.secondary_width_pixels, 640u);
    EXPECT_EQ(v1.secondary_height_pixels, 368u);
    EXPECT_EQ(v1.primary_line_stride_bytes, 1280u);
    EXPECT_EQ(v1.secondary_line_stride_bytes, 1280u);
    EXPECT_EQ(v1.primary_start_offset, 0u);
    EXPECT_EQ(v1.secondary_start_offset, 1280u * 736u);
    EXPECT_EQ(v1.tertiary_start_offset, 1280u * 736u + 1u);
    EXPECT_EQ(v1.primary_pixel_stride, 1u);
    EXPECT_EQ(v1.secondary_pixel_stride, 2u);
    EXPECT_EQ(v1.primary_display_width_pixels, 1280u);
    EXPECT_EQ(v1.primary_display_height_pixels, 720u);
    EXPECT_TRUE(v1.has_pixel_aspect_ratio);
    EXPECT_EQ(v1.pixel_aspect_ratio_width, 16u);
    EXPECT_EQ(v1.pixel_aspect_ratio_height, 9u);
    EXPECT_EQ(v1.image_format.pixel_format.type, fuchsia::sysmem::PixelFormatType::NV12);
    EXPECT_EQ(v1.image_format.coded_width, 1280u);
    EXPECT_EQ(v1.image_format.coded_height, 736u);
    EXPECT_EQ(v1.image_format.bytes_per_row, 1280u);
    EXPECT_EQ(v1.image_format.display_width, 1280u);
    EXPECT_EQ(v1.image_format.display_height, 720u);
  }

  {
    fuchsia::media::FormatDetails format_details;
    format_details.set_format_details_version_ordinal(2);
    format_details.set_mime_type("video/raw");

    auto* v2 = format_details.mutable_image_format();
    v2->set_pixel_format(fuchsia::images2::PixelFormat::YV12);
    v2->set_size({.width = 640, .height = 368});
    v2->set_bytes_per_row(640);
    v2->set_display_rect({.x = 0, .y = 0, .width = 640, .height = 360});
    v2->set_color_space(fuchsia::images2::ColorSpace::REC709);

    CodecImpl::EnsureFormatDetailsBridged(&format_details);

    ASSERT_TRUE(format_details.has_domain());
    const auto& v1 = format_details.domain().video().uncompressed();
    EXPECT_EQ(v1.fourcc, make_fourcc('Y', 'V', '1', '2'));
    EXPECT_TRUE(v1.planar);
    EXPECT_FALSE(v1.swizzled);
    EXPECT_EQ(v1.primary_width_pixels, 640u);
    EXPECT_EQ(v1.primary_height_pixels, 368u);
    EXPECT_EQ(v1.secondary_width_pixels, 320u);
    EXPECT_EQ(v1.secondary_height_pixels, 184u);
    EXPECT_EQ(v1.primary_line_stride_bytes, 640u);
    EXPECT_EQ(v1.secondary_line_stride_bytes, 320u);
    EXPECT_EQ(v1.primary_start_offset, 0u);
    EXPECT_EQ(v1.secondary_start_offset, 640u * 368u);
    EXPECT_EQ(v1.tertiary_start_offset, 640u * 368u + 320u * 184u);
    EXPECT_EQ(v1.primary_pixel_stride, 1u);
    EXPECT_EQ(v1.secondary_pixel_stride, 1u);
    EXPECT_EQ(v1.primary_display_width_pixels, 640u);
    EXPECT_EQ(v1.primary_display_height_pixels, 360u);
    EXPECT_FALSE(v1.has_pixel_aspect_ratio);

    // Mutating `image_format` on an already-bridged `FormatDetails` updates `domain`.
    format_details.mutable_image_format()->set_size({.width = 1920, .height = 1088});
    format_details.mutable_image_format()->set_bytes_per_row(1920);
    format_details.mutable_image_format()->set_display_rect(
        {.x = 0, .y = 0, .width = 1920, .height = 1080});
    CodecImpl::EnsureFormatDetailsBridged(&format_details);
    EXPECT_EQ(format_details.domain().video().uncompressed().primary_width_pixels, 1920u);
    EXPECT_EQ(format_details.domain().video().uncompressed().primary_display_height_pixels, 1080u);
  }

  {
    // Overflowing dimensions must not panic or wrap around.
    fuchsia::media::FormatDetails overflow_details;
    overflow_details.set_format_details_version_ordinal(3);
    auto* v2 = overflow_details.mutable_image_format();
    v2->set_pixel_format(fuchsia::images2::PixelFormat::NV12);
    v2->set_pixel_format_modifier(fuchsia::images2::PixelFormatModifier::INTEL_I915_Y_TILED);
    v2->set_size({.width = 65536, .height = 0xFFFFFFF0u});
    v2->set_bytes_per_row(65536);

    CodecImpl::EnsureFormatDetailsBridged(&overflow_details);
    EXPECT_FALSE(overflow_details.has_domain());
  }

  {
    // Compressed video `mime_type` synthesizes `domain.video.compressed`.
    fuchsia::media::FormatDetails compressed_video;
    compressed_video.set_format_details_version_ordinal(4);
    compressed_video.set_mime_type("video/h264");

    CodecImpl::EnsureFormatDetailsBridged(&compressed_video);
    ASSERT_TRUE(compressed_video.has_domain());
    ASSERT_TRUE(compressed_video.domain().is_video());
    EXPECT_TRUE(compressed_video.domain().video().is_compressed());
  }

  {
    // Linear NV12 and I420 `ImageFormat` omitting optional `bytes_per_row` (and omitting
    // `display_rect` while setting `valid_size`) bridges to unpadded V1 strides and non-overlapping
    // UV plane offsets.
    fuchsia::media::FormatDetails nv12_unpadded;
    nv12_unpadded.set_format_details_version_ordinal(5);
    nv12_unpadded.set_mime_type("video/raw");
    auto* nv12_v2 = nv12_unpadded.mutable_image_format();
    nv12_v2->set_pixel_format(fuchsia::images2::PixelFormat::NV12);
    nv12_v2->set_size({.width = 640, .height = 368});
    nv12_v2->set_valid_size({.width = 640, .height = 360});

    CodecImpl::EnsureFormatDetailsBridged(&nv12_unpadded);
    ASSERT_TRUE(nv12_unpadded.has_domain());
    const auto& nv12_v1 = nv12_unpadded.domain().video().uncompressed();
    EXPECT_FALSE(nv12_unpadded.image_format().has_bytes_per_row());
    EXPECT_EQ(nv12_v1.image_format.bytes_per_row, 640u);
    EXPECT_EQ(nv12_v1.primary_line_stride_bytes, 640u);
    EXPECT_EQ(nv12_v1.secondary_line_stride_bytes, 640u);
    EXPECT_EQ(nv12_v1.secondary_start_offset, 640u * 368u);
    EXPECT_EQ(nv12_v1.tertiary_start_offset, 640u * 368u + 1u);
    EXPECT_EQ(nv12_v1.primary_display_width_pixels, 640u);
    EXPECT_EQ(nv12_v1.primary_display_height_pixels, 360u);

    fuchsia::media::FormatDetails nv12_bridged_once = fidl::Clone(nv12_unpadded);
    CodecImpl::EnsureFormatDetailsBridged(&nv12_unpadded);
    EXPECT_TRUE(fidl::Equals(nv12_bridged_once, nv12_unpadded));

    fuchsia::media::FormatDetails i420_unpadded;
    i420_unpadded.set_format_details_version_ordinal(6);
    i420_unpadded.set_mime_type("video/raw");
    auto* i420_v2 = i420_unpadded.mutable_image_format();
    i420_v2->set_pixel_format(fuchsia::images2::PixelFormat::I420);
    i420_v2->set_pixel_format_modifier(fuchsia::images2::PixelFormatModifier::LINEAR);
    i420_v2->set_size({.width = 640, .height = 368});

    CodecImpl::EnsureFormatDetailsBridged(&i420_unpadded);
    ASSERT_TRUE(i420_unpadded.has_domain());
    const auto& i420_v1 = i420_unpadded.domain().video().uncompressed();
    EXPECT_EQ(i420_v1.primary_line_stride_bytes, 640u);
    EXPECT_EQ(i420_v1.secondary_line_stride_bytes, 320u);
    EXPECT_EQ(i420_v1.secondary_start_offset, 640u * 368u);
    EXPECT_EQ(i420_v1.tertiary_start_offset, 640u * 368u + 320u * 184u);
  }
}

TEST(CodecImplFormatDetailsBridge, VideoV1ToV2OptionalFieldOmission) {
  {
    // Display-only V1 metadata with default/zero coded dimensions, stride, and INVALID color space
    // must leave `color_space`, `bytes_per_row`, `size`, and `valid_size` unset on V2
    // `image_format`.
    fuchsia::media::FormatDetails format_details;
    format_details.set_format_details_version_ordinal(1);
    format_details.set_mime_type("video/raw");

    auto& uncompressed = format_details.mutable_domain()->video().uncompressed();
    uncompressed.image_format.pixel_format.type = fuchsia::sysmem::PixelFormatType::NV12;
    uncompressed.image_format.coded_width = 0;
    uncompressed.image_format.coded_height = 0;
    uncompressed.image_format.bytes_per_row = 0;
    uncompressed.image_format.display_width = 640;
    uncompressed.image_format.display_height = 480;
    uncompressed.image_format.color_space.type = fuchsia::sysmem::ColorSpaceType::INVALID;

    CodecImpl::EnsureFormatDetailsBridged(&format_details);

    ASSERT_TRUE(format_details.has_image_format());
    const auto& v2 = format_details.image_format();
    ASSERT_TRUE(v2.has_pixel_format());
    EXPECT_EQ(v2.pixel_format(), fuchsia::images2::PixelFormat::NV12);
    EXPECT_FALSE(v2.has_color_space());
    EXPECT_FALSE(v2.has_bytes_per_row());
    EXPECT_FALSE(v2.has_size());
    EXPECT_FALSE(v2.has_valid_size());
    ASSERT_TRUE(v2.has_display_rect());
    EXPECT_EQ(v2.display_rect().x, 0u);
    EXPECT_EQ(v2.display_rect().y, 0u);
    EXPECT_EQ(v2.display_rect().width, 640u);
    EXPECT_EQ(v2.display_rect().height, 480u);

    fuchsia::media::FormatDetails bridged_once = fidl::Clone(format_details);
    CodecImpl::EnsureFormatDetailsBridged(&format_details);
    EXPECT_TRUE(fidl::Equals(bridged_once, format_details));
  }

  {
    // Legacy V1 encoder input omitting `pixel_format`, `color_space`, and `bytes_per_row` while
    // specifying coded dimensions leaves those optional V2 fields unset.
    fuchsia::media::FormatDetails format_details;
    format_details.set_format_details_version_ordinal(2);
    format_details.set_mime_type("video/raw");

    auto& uncompressed = format_details.mutable_domain()->video().uncompressed();
    uncompressed.image_format.coded_width = 1280;
    uncompressed.image_format.coded_height = 720;
    uncompressed.image_format.bytes_per_row = 0;
    uncompressed.image_format.color_space.type = fuchsia::sysmem::ColorSpaceType::INVALID;

    CodecImpl::EnsureFormatDetailsBridged(&format_details);

    ASSERT_TRUE(format_details.has_image_format());
    const auto& v2 = format_details.image_format();
    EXPECT_FALSE(v2.has_pixel_format());
    EXPECT_FALSE(v2.has_color_space());
    EXPECT_FALSE(v2.has_bytes_per_row());
    EXPECT_FALSE(v2.has_display_rect());
    ASSERT_TRUE(v2.has_size());
    EXPECT_EQ(v2.size().width, 1280u);
    EXPECT_EQ(v2.size().height, 720u);
  }
}

TEST(CodecImplFormatDetailsBridge, ClearsStaleDomainOnV2OnlyMutation) {
  // Video: start from a bridged V2+V1 FormatDetails, then mutate `image_format` to V2-only values.
  {
    fuchsia::media::FormatDetails base_video;
    base_video.set_format_details_version_ordinal(1);
    base_video.set_mime_type("video/raw");
    auto* v2 = base_video.mutable_image_format();
    v2->set_pixel_format(fuchsia::images2::PixelFormat::NV12);
    v2->set_size({.width = 1280, .height = 720});
    v2->set_bytes_per_row(1280);
    v2->set_display_rect({.x = 0, .y = 0, .width = 1280, .height = 720});
    CodecImpl::EnsureFormatDetailsBridged(&base_video);
    ASSERT_TRUE(base_video.has_domain());

    // Non-zero `display_rect.x` clears stale `domain`.
    fuchsia::media::FormatDetails cropped_x = fidl::Clone(base_video);
    cropped_x.mutable_image_format()->set_display_rect(
        {.x = 4, .y = 0, .width = 1276, .height = 720});
    CodecImpl::EnsureFormatDetailsBridged(&cropped_x);
    EXPECT_FALSE(cropped_x.has_domain());
    EXPECT_TRUE(cropped_x.has_image_format());

    // Non-zero `display_rect.y` clears stale `domain`.
    fuchsia::media::FormatDetails cropped_y = fidl::Clone(base_video);
    cropped_y.mutable_image_format()->set_display_rect(
        {.x = 0, .y = 2, .width = 1280, .height = 718});
    CodecImpl::EnsureFormatDetailsBridged(&cropped_y);
    EXPECT_FALSE(cropped_y.has_domain());

    // V2-only `PixelFormat::P010` clears stale `domain`.
    fuchsia::media::FormatDetails p010 = fidl::Clone(base_video);
    p010.mutable_image_format()->set_pixel_format(fuchsia::images2::PixelFormat::P010);
    CodecImpl::EnsureFormatDetailsBridged(&p010);
    EXPECT_FALSE(p010.has_domain());

    // V2-only `ColorSpace` clears stale `domain`.
    fuchsia::media::FormatDetails unknown_cs = fidl::Clone(base_video);
    unknown_cs.mutable_image_format()->set_color_space(
        static_cast<fuchsia::images2::ColorSpace>(100));
    CodecImpl::EnsureFormatDetailsBridged(&unknown_cs);
    EXPECT_FALSE(unknown_cs.has_domain());
  }
}
