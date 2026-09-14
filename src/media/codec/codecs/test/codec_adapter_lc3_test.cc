// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include <fuchsia/media/cpp/fidl.h>

#include <cstdarg>
#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <mutex>
#include <string>
#include <utility>
#include <vector>

#include <gtest/gtest.h>

#include "src/media/codec/codecs/sw/lc3/codec_adapter_lc3_decoder.h"
#include "src/media/codec/codecs/sw/lc3/codec_adapter_lc3_encoder.h"

namespace {

class FakeCodecAdapterEvents : public CodecAdapterEvents {
 public:
  void onCoreCodecFailCodec(const char* format, ...) override {
    fail_codec_count_++;
    char buffer[256];
    va_list args;
    va_start(args, format);
    std::vsnprintf(buffer, sizeof(buffer), format, args);
    va_end(args);
    last_fail_message_ = buffer;
  }

  void onCoreCodecFailStream(fuchsia::media::StreamError error) override {}
  void onCoreCodecResetStreamAfterCurrentFrame() override {}
  void onCoreCodecMidStreamOutputConstraintsChange(bool output_re_config_required) override {}
  void onCoreCodecOutputFormatChange() override {}
  void onCoreCodecInputPacketDone(CodecPacket* packet) override {}
  void onCoreCodecOutputPacket(CodecPacket* packet, bool error_detected_before,
                               bool error_detected_during) override {}
  void onCoreCodecOutputTimestampHasNoOutput(uint64_t timestamp_ish) override {}
  void onCoreCodecOutputEndOfStream(bool error_detected_before) override {}
  void onCoreCodecLogEvent(
      media_metrics::StreamProcessorEvents2MigratedMetricDimensionEvent event_code) override {}

  size_t fail_codec_count() const { return fail_codec_count_; }
  const std::string& last_fail_message() const { return last_fail_message_; }

 private:
  size_t fail_codec_count_ = 0;
  std::string last_fail_message_;
};

class TestCodecAdapterLc3Decoder : public CodecAdapterLc3Decoder {
 public:
  TestCodecAdapterLc3Decoder(std::mutex& lock, CodecAdapterEvents* events)
      : CodecAdapterLc3Decoder(lock, events) {}

  using CodecAdapterLc3Decoder::InputChunkSize;
  using CodecAdapterLc3Decoder::InputLoopStatus;
  using CodecAdapterLc3Decoder::MinOutputBufferSize;
  using CodecAdapterLc3Decoder::OutputFormatDetails;
  using CodecAdapterLc3Decoder::ProcessFormatDetails;
  using CodecAdapterLc3Decoder::ProcessInputChunkData;
};

class TestCodecAdapterLc3Encoder : public CodecAdapterLc3Encoder {
 public:
  TestCodecAdapterLc3Encoder(std::mutex& lock, CodecAdapterEvents* events)
      : CodecAdapterLc3Encoder(lock, events) {}

  using CodecAdapterLc3Encoder::CreateTimestampExtrapolator;
  using CodecAdapterLc3Encoder::InputChunkSize;
  using CodecAdapterLc3Encoder::InputLoopStatus;
  using CodecAdapterLc3Encoder::MinOutputBufferSize;
  using CodecAdapterLc3Encoder::ProcessFormatDetails;
  using CodecAdapterLc3Encoder::ProcessInputChunkData;
};

fuchsia::media::FormatDetails MakeLc3FormatDetails(std::vector<uint8_t> oob_bytes) {
  fuchsia::media::FormatDetails format_details;
  format_details.set_mime_type(kLc3MimeType);
  format_details.set_oob_bytes(std::move(oob_bytes));
  return format_details;
}

fuchsia::media::FormatDetails MakeValidLc3EncoderFormatDetails() {
  fuchsia::media::PcmFormat pcm;
  pcm.pcm_mode = fuchsia::media::AudioPcmMode::LINEAR;
  pcm.bits_per_sample = 16;
  pcm.frames_per_second = 48000;
  pcm.channel_map = {fuchsia::media::AudioChannelId::LF};

  fuchsia::media::AudioUncompressedFormat uncompressed;
  uncompressed.set_pcm(std::move(pcm));

  fuchsia::media::AudioFormat audio;
  audio.set_uncompressed(std::move(uncompressed));

  fuchsia::media::DomainFormat domain;
  domain.set_audio(std::move(audio));

  fuchsia::media::Lc3EncoderSettings lc3_settings;
  lc3_settings.set_nbytes(40);
  lc3_settings.set_frame_duration(fuchsia::media::Lc3FrameDuration::D10_MS);

  fuchsia::media::EncoderSettings encoder_settings;
  encoder_settings.set_lc3(std::move(lc3_settings));

  fuchsia::media::FormatDetails format_details;
  format_details.set_domain(std::move(domain));
  format_details.set_encoder_settings(std::move(encoder_settings));
  return format_details;
}

TEST(CodecAdapterLc3DecoderTest, ValidOobBytesSucceeds) {
  std::mutex lock;
  FakeCodecAdapterEvents events;
  TestCodecAdapterLc3Decoder decoder(lock, &events);

  // Valid 16-byte LTV configuration:
  // Sampling_Frequency: 48 kHz (0x08)
  // Frame_Duration: 10 ms (0x01)
  // Audio_Channel_Allocation: LF (0x00000001)
  // Octets_Per_Codec_Frame: 40 (0x0028)
  std::vector<uint8_t> oob_bytes = {
      0x02, 0x01, 0x08,                    // Sampling_Frequency
      0x02, 0x02, 0x01,                    // Frame_Duration
      0x05, 0x03, 0x00, 0x00, 0x00, 0x01,  // Audio_Channel_Allocation
      0x03, 0x04, 0x00, 0x28               // Octets_Per_Codec_Frame
  };

  auto status = decoder.ProcessFormatDetails(MakeLc3FormatDetails(std::move(oob_bytes)));
  EXPECT_EQ(status, TestCodecAdapterLc3Decoder::kOk);
  EXPECT_EQ(events.fail_codec_count(), 0u);
  EXPECT_EQ(decoder.InputChunkSize(), 40u);
  EXPECT_EQ(decoder.MinOutputBufferSize(), 960u);
}

TEST(CodecAdapterLc3DecoderTest, MissingMimeTypeOrOobBytesRejected) {
  std::mutex lock;
  FakeCodecAdapterEvents events;
  TestCodecAdapterLc3Decoder decoder(lock, &events);

  fuchsia::media::FormatDetails missing_mime;
  missing_mime.set_oob_bytes(std::vector<uint8_t>(16, 0));
  EXPECT_EQ(decoder.ProcessFormatDetails(missing_mime),
            TestCodecAdapterLc3Decoder::kShouldTerminate);
  EXPECT_EQ(events.fail_codec_count(), 1u);

  fuchsia::media::FormatDetails short_oob;
  short_oob.set_mime_type(kLc3MimeType);
  short_oob.set_oob_bytes(std::vector<uint8_t>(15, 0));
  EXPECT_EQ(decoder.ProcessFormatDetails(short_oob), TestCodecAdapterLc3Decoder::kShouldTerminate);
  EXPECT_EQ(events.fail_codec_count(), 2u);
}

TEST(CodecAdapterLc3DecoderTest, OutOfRangeOctetsPerCodecFrameRejected) {
  std::mutex lock;
  FakeCodecAdapterEvents events;
  TestCodecAdapterLc3Decoder decoder(lock, &events);

  // Octets_Per_Codec_Frame = 19 (< kMinExternalByteCount = 20).
  std::vector<uint8_t> oob_bytes = {
      0x02, 0x01, 0x08,                    // Sampling_Frequency
      0x02, 0x02, 0x01,                    // Frame_Duration
      0x05, 0x03, 0x00, 0x00, 0x00, 0x01,  // Audio_Channel_Allocation
      0x03, 0x04, 0x00, 0x13               // Octets_Per_Codec_Frame = 19
  };

  auto status = decoder.ProcessFormatDetails(MakeLc3FormatDetails(std::move(oob_bytes)));
  EXPECT_EQ(status, TestCodecAdapterLc3Decoder::kShouldTerminate);
  EXPECT_EQ(events.fail_codec_count(), 1u);
  EXPECT_NE(events.last_fail_message().find("Octets_Per_Codec_Frame"), std::string::npos)
      << "Actual message: " << events.last_fail_message();
}

TEST(CodecAdapterLc3EncoderTest, ValidFormatDetailsSucceeds) {
  std::mutex lock;
  FakeCodecAdapterEvents events;
  TestCodecAdapterLc3Encoder encoder(lock, &events);

  auto status = encoder.ProcessFormatDetails(MakeValidLc3EncoderFormatDetails());
  EXPECT_EQ(status, TestCodecAdapterLc3Encoder::kOk);
  EXPECT_EQ(events.fail_codec_count(), 0u);
  EXPECT_EQ(encoder.InputChunkSize(), 960u);
  EXPECT_EQ(encoder.MinOutputBufferSize(), 40u);
}

TEST(CodecAdapterLc3EncoderTest, MissingDomainOrEncoderSettingsRejected) {
  std::mutex lock;
  FakeCodecAdapterEvents events;
  TestCodecAdapterLc3Encoder encoder(lock, &events);

  fuchsia::media::FormatDetails missing_domain = MakeValidLc3EncoderFormatDetails();
  missing_domain.clear_domain();
  EXPECT_EQ(encoder.ProcessFormatDetails(missing_domain),
            TestCodecAdapterLc3Encoder::kShouldTerminate);
  EXPECT_EQ(events.fail_codec_count(), 1u);

  fuchsia::media::FormatDetails missing_settings = MakeValidLc3EncoderFormatDetails();
  missing_settings.clear_encoder_settings();
  EXPECT_EQ(encoder.ProcessFormatDetails(missing_settings),
            TestCodecAdapterLc3Encoder::kShouldTerminate);
  EXPECT_EQ(events.fail_codec_count(), 2u);
}

TEST(CodecAdapterLc3EncoderTest, OutOfRangeEncoderNbytesRejected) {
  std::mutex lock;
  FakeCodecAdapterEvents events;
  TestCodecAdapterLc3Encoder encoder(lock, &events);

  auto format_details = MakeValidLc3EncoderFormatDetails();
  format_details.mutable_encoder_settings()->lc3().set_nbytes(401);

  auto status = encoder.ProcessFormatDetails(format_details);
  EXPECT_EQ(status, TestCodecAdapterLc3Encoder::kShouldTerminate);
  EXPECT_EQ(events.fail_codec_count(), 1u);
  EXPECT_NE(events.last_fail_message().find("Byte count"), std::string::npos)
      << "Actual message: " << events.last_fail_message();
}

}  // namespace
