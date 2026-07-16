// Copyright 2023 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#ifndef ZIRCON_KERNEL_LIB_THREAD_SAMPLER_INCLUDE_LIB_THREAD_SAMPLER_PER_CPU_STATE_H_
#define ZIRCON_KERNEL_LIB_THREAD_SAMPLER_INCLUDE_LIB_THREAD_SAMPLER_PER_CPU_STATE_H_
#include <lib/percpu_writer/buffer.h>
#include <zircon/syscalls-next.h>

// These types are internal implementation details to the thread sampler.
namespace sampler::internal {

class PerCpuState {
 public:
  constexpr PerCpuState() = default;

  zx::result<> SetUp(const zx_sampler_config_t& config, cpu_num_t cpu_number);

  // Reserve space in the assigned pinned memory. The AllocatedRecord will ensure the underlying
  // buffers live long enough to write to.
  zx::result<percpu_writer::Buffer::Reservation> Reserve(uint64_t header) {
    return writer.Reserve(header);
  }

  // Reads from the underlying SpscBuffer.
  template <CopyOutFunction CopyFunc>
  zx::result<uint32_t> Read(CopyFunc copy_fn, uint32_t len) {
    return writer.Read(copy_fn, len);
  }

  size_t BufferSize() const { return writer.Size(); }
  void Drain() { writer.Drain(); }

 private:
  percpu_writer::Buffer writer;
};

}  // namespace sampler::internal
#endif  // ZIRCON_KERNEL_LIB_THREAD_SAMPLER_INCLUDE_LIB_THREAD_SAMPLER_PER_CPU_STATE_H_
