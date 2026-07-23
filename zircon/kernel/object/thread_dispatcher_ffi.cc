// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include <object/thread_dispatcher.h>

extern "C" {

bool cpp_thread_dispatcher_is_current(const ThreadDispatcher* thread) {
  return thread == ThreadDispatcher::GetCurrent();
}

zx_status_t cpp_thread_dispatcher_suspend(ThreadDispatcher* thread) { return thread->Suspend(); }

void cpp_thread_dispatcher_resume(ThreadDispatcher* thread) { thread->Resume(); }

}  // extern "C"
