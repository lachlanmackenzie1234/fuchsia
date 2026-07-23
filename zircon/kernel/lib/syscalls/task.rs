// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use debug::ltracef;
use object::{Dispatcher, HandleValue, ProcessDispatcher, SuspendTokenDispatcher};
use syscalls_macro::syscall;
use zx_status::ErrorStatus;
use zx_types::ZX_RIGHT_WRITE;

const LOCAL_TRACE: u32 = 0;

#[syscall]
pub fn sys_task_suspend(handle: HandleValue, token: &mut HandleValue) -> Result<(), ErrorStatus> {
    ltracef!("handle {:#x}\n", handle.raw_value());

    let task = Dispatcher::get_with_rights::<Dispatcher>(handle, ZX_RIGHT_WRITE)?;
    let (new_token, rights) = SuspendTokenDispatcher::create(task)?;
    *token = ProcessDispatcher::with_current(|up| up.make_and_add_handle(new_token, rights))?;
    Ok(())
}

#[syscall]
pub fn sys_task_suspend_token(
    handle: HandleValue,
    token: &mut HandleValue,
) -> Result<(), ErrorStatus> {
    sys_task_suspend(handle, token)
}
