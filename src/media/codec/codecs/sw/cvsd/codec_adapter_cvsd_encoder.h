// Copyright 2022 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#ifndef SRC_MEDIA_CODEC_CODECS_SW_CVSD_CODEC_ADAPTER_CVSD_ENCODER_H_
#define SRC_MEDIA_CODEC_CODECS_SW_CVSD_CODEC_ADAPTER_CVSD_ENCODER_H_

#include <lib/media/codec_impl/codec_adapter.h>

#include "chunk_input_stream.h"
#include "codec_adapter_cvsd.h"
#include "codec_adapter_sw.h"

class CodecAdapterCvsdEncoder : public CodecAdapterSW<fit::deferred_action<fit::closure>> {
 public:
  explicit CodecAdapterCvsdEncoder(std::mutex& lock, CodecAdapterEvents* codec_adapter_events);
  ~CodecAdapterCvsdEncoder() = default;

  fuchsia::sysmem::BufferCollectionConstraints CoreCodecGetBufferCollectionConstraints(
      CodecPort port, const fuchsia::media::StreamBufferConstraints& stream_buffer_constraints,
      const fuchsia::media::StreamBufferPartialSettings& partial_settings) override;

  void CoreCodecSetBufferCollectionInfo(
      CodecPort port,
      const fuchsia::sysmem::BufferCollectionInfo_2& buffer_collection_info) override;

  void CoreCodecStopStream() override;

 protected:
  void ProcessInputLoop() override;
  void CleanUpAfterStream() override;

  // Returns the format details of the output and the bytes needed to store each
  // output packet.
  std::pair<fuchsia::media::FormatDetails, size_t> OutputFormatDetails() override;

 private:
  InputLoopStatus ProcessFormatDetails(const fuchsia::media::FormatDetails& format_details);
  InputLoopStatus ProcessEndOfStream(CodecInputItem* item);
  InputLoopStatus ProcessInputPacket(CodecPacket* packet);
  InputLoopStatus ProcessCodecPacket(CodecPacket* packet);
  // Outputs and resets current output buffer.
  void SendAndResetOutputPacket();
  void InitChunkInputStream(const fuchsia::media::FormatDetails& format_details);

  // Encodes a single output byte.
  static void Encode(CodecParams* params, const uint8_t* input_data, uint8_t* output);

  std::optional<CodecParams> codec_params_;

  // Current input buffer where we buffer data from the input packet
  // before sending it over to be encoded as output. CVSD encoder processes
  // 2 bytes of data to produce 1 bit of output data. Hence, the buffer should
  // be initialized to 16 bytes size.
  std::optional<ChunkInputStream> chunk_input_stream_;

  // Current output item that we are currently encoding into.
  std::optional<OutputItem> output_item_;
};

#endif  // SRC_MEDIA_CODEC_CODECS_SW_CVSD_CODEC_ADAPTER_CVSD_ENCODER_H_
