// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include <lib/boot-options/boot-options.h>
#include <lib/thread_sampler/thread_sampler.h>
#include <zircon/rights.h>

#include <fbl/alloc_checker.h>
#include <fbl/ref_ptr.h>
#include <ktl/utility.h>
#include <object/handle.h>
#include <object/sampler_dispatcher.h>

#ifdef EXPERIMENTAL_THREAD_SAMPLER_ENABLED
constexpr bool kSamplerEnabled = EXPERIMENTAL_THREAD_SAMPLER_ENABLED;
#else
// The build system should always define the macro.
#error
#endif

extern "C" {

zx_status_t cpp_sampler_dispatcher_create(const zx_sampler_config_t* config,
                                          KernelHandle<SamplerDispatcher>* handle_out) {
  // Set up the global sampler if it hasn't been set up yet.
  zx::result res = sampler::gThreadSampler.SetUp(*config);
  if (res.is_error()) {
    return res.status_value();
  }

  fbl::AllocChecker ac;
  auto disp = fbl::AdoptRef(new (&ac) SamplerDispatcher());
  if (!ac.check()) {
    return ZX_ERR_NO_MEMORY;
  }

  new (handle_out) KernelHandle<SamplerDispatcher>(ktl::move(disp));
  return ZX_OK;
}

zx_status_t cpp_sampler_dispatcher_start(const SamplerDispatcher* dispatcher) {
  return sampler::gThreadSampler.Start().status_value();
}

zx_status_t cpp_sampler_dispatcher_stop(const SamplerDispatcher* dispatcher) {
  return sampler::gThreadSampler.Stop().status_value();
}

zx_status_t cpp_sampler_dispatcher_read_user(const SamplerDispatcher* dispatcher, void* ptr,
                                             size_t len, size_t* actual_out) {
  // We unfortunately run into some complexity here: while the buffer our samples in is created by
  // the kernel and is safe to read from, the user memory we are writing to could be pager-backed.
  // This means that when we attempt to write to it as part of the VmObjectPaged::ReadUser call, we
  // cannot be holding locks. So we need to obtain the lock to set up the copy, drop the lock, do
  // the copy, then grab the lock again to make sure everything went well.
  //
  // During the copy, we'd need to prevent:
  //   1) The sampler from writing new data
  //   2) The buffers being destroyed due to the read handle being zx_handle_close'd
  //   3) A new sampler from being created.
  //
  // We do this by:
  //    1) Setting our state to SamplingState::Reading which disallows starting a new session (and
  //       thus destroying the old one).
  //    2) If on_zero_handles is triggered while in `Reading` mode, we delay actually
  //       destroying the buffers and destroy them after the copy is completed instead.
  zx::result<sampler::ReadToken> token = sampler::gThreadSampler.PrepareRead();
  if (token.is_error()) {
    *actual_out = 0;
    return token.error_value();
  }

  auto [status, read] = sampler::gThreadSampler.ReadUser(*token, user_out_ptr<void>(ptr), len);
  *actual_out = read;

  // We now need to ensure that the user side handle hasn't been dropped. If it has been, then
  // we need to clean it up.
  sampler::gThreadSampler.FinishRead(ktl::move(*token));
  return status;
}

bool cpp_sampler_enabled() { return kSamplerEnabled; }

}  // extern "C"
