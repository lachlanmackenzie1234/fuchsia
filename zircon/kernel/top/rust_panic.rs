// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use core::ffi::{c_char, c_int};
use core::panic::PanicInfo;

unsafe extern "C" {
    fn panic(fmt: *const c_char, line: c_int, ...) -> !;
}

#[panic_handler]
fn rust_panic(info: &PanicInfo<'_>) -> ! {
    let msg_str = {
        if let Some(msg_str) = info.message().as_str() {
            msg_str
        } else {
            "non-static Rust panic message"
        }
    };

    if let Some(location) = info.location() {
        let file = location.file();
        let line = location.line();
        unsafe {
            panic(
                c"%.*s:%d: %.*s".as_ptr(),
                file.len() as c_int,
                file.as_ptr(),
                line as c_int,
                msg_str.len() as c_int,
                msg_str.as_ptr(),
            );
        }
    } else {
        unsafe {
            panic(c"%.*s".as_ptr(), msg_str.len() as i32, msg_str.as_ptr());
        }
    }
}
