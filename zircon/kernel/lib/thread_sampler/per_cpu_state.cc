// Copyright 2019 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include <lib/thread_sampler/per_cpu_state.h>

namespace sampler::internal {
zx::result<> PerCpuState::SetUp(const zx_sampler_config_t& config, cpu_num_t cpu_number) {
  // TODO(https://fxbug.dev/377907138) Allow configuration of the per cpu buffer size.
  size_t buffer_size = size_t{4} * 1024 * 1024;
  return zx::make_result(writer.Init(static_cast<uint32_t>(buffer_size), "sampler",
                                     fxt::ThreadRef{fxt::Koid{0}, fxt::Koid{cpu_number}}));
}

}  // namespace sampler::internal
