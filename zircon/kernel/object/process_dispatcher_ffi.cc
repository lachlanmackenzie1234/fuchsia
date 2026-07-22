// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include <object/handle.h>
#include <object/process_dispatcher.h>

extern "C" {

ProcessDispatcher* cpp_process_dispatcher_current() { return ProcessDispatcher::GetCurrent(); }

zx_status_t cpp_process_dispatcher_make_and_add_handle(ProcessDispatcher* process,
                                                       KernelHandle<Dispatcher>* handle,
                                                       zx_rights_t rights,
                                                       zx_handle_t* out_handle) {
  return process->MakeAndAddHandle(ktl::move(*handle), rights, out_handle);
}

zx_status_t cpp_handle_table_get_dispatcher(zx_handle_t handle, fbl::RefPtr<Dispatcher>* out_disp,
                                            zx_rights_t* out_rights) {
  auto up = ProcessDispatcher::GetCurrent();
  return up->handle_table().GetDispatcherAndRights(*up, handle, out_disp, out_rights);
}

zx_status_t cpp_process_dispatcher_enforce_basic_policy(const ProcessDispatcher* process,
                                                        uint32_t policy) {
  return const_cast<ProcessDispatcher*>(process)->EnforceBasicPolicy(policy);
}

}  // extern "C"
