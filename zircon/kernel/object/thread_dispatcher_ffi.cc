// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include <lib/object-constants.h>

#include <object/thread_dispatcher.h>

extern "C" {
bool cpp_thread_dispatcher_is_current(const ThreadDispatcher* thread);
zx_status_t cpp_thread_dispatcher_suspend(ThreadDispatcher* thread);
void cpp_thread_dispatcher_resume(ThreadDispatcher* thread);
zx_status_t cpp_thread_dispatcher_set_base_profile(ThreadDispatcher* thread,
                                                   const SchedulerState::BaseProfile* profile);
zx_status_t cpp_thread_dispatcher_set_soft_affinity(ThreadDispatcher* thread, cpu_mask_t mask);

bool cpp_thread_dispatcher_is_current(const ThreadDispatcher* thread) {
  return thread == ThreadDispatcher::GetCurrent();
}

zx_status_t cpp_thread_dispatcher_suspend(ThreadDispatcher* thread) { return thread->Suspend(); }

void cpp_thread_dispatcher_resume(ThreadDispatcher* thread) { thread->Resume(); }

zx_status_t cpp_thread_dispatcher_set_base_profile(ThreadDispatcher* thread,
                                                   const SchedulerState::BaseProfile* profile) {
  static_assert(sizeof(SchedulerState::BaseProfile) == kSchedulerStateBaseProfileSize,
                "SchedulerState::BaseProfile size mismatch");
  static_assert(alignof(SchedulerState::BaseProfile) == kSchedulerStateBaseProfileAlign,
                "SchedulerState::BaseProfile align mismatch");
  return thread->SetBaseProfile(*profile);
}

zx_status_t cpp_thread_dispatcher_set_soft_affinity(ThreadDispatcher* thread, cpu_mask_t mask) {
  return thread->SetSoftAffinity(mask);
}

}  // extern "C"
