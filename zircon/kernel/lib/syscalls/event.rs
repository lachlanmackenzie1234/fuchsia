// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

use crate::object::{EventDispatcher, HandleValue, ProcessDispatcher};
use debug::ltracef;
use syscalls_macro::syscall;
use zx_status::{ErrorStatus, Status};
use zx_types::ZX_POL_NEW_EVENT;

const LOCAL_TRACE: u32 = 0;

#[syscall]
pub fn sys_event_create(options: u32, out: &mut HandleValue) -> Result<(), ErrorStatus> {
    ltracef!("options {:#x}\n", options);

    if options != 0 {
        return Err(Status::INVALID_ARGS.into());
    }

    ProcessDispatcher::with_current(|up| up.enforce_basic_policy(ZX_POL_NEW_EVENT))?;

    let (kernel_handle, rights) = EventDispatcher::create(options)?;
    let user_handle = kernel_handle.make_and_add_handle(rights)?;
    *out = user_handle;
    Ok(())
}
