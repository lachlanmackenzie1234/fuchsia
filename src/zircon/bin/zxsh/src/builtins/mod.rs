// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::eval::{EvalOutcome, ExecutionContext, ShellState};
use crate::string::parse_int;
use bstr::{BStr, BString, ByteSlice};

pub fn is_builtin(name: &BStr) -> bool {
    matches!(
        name.as_bytes(),
        b"true" | b"false" | b":" | b"exit" | b"return" | b"break" | b"continue"
    )
}

pub fn run_builtin(
    name: &BStr,
    args: &[BString],
    state: &mut ShellState,
    _ctx: &mut ExecutionContext,
) -> Result<EvalOutcome, String> {
    match name.as_bytes() {
        b"true" | b":" => Ok(EvalOutcome::Code(0)),
        b"false" => Ok(EvalOutcome::Code(1)),
        b"exit" => {
            let code = if args.is_empty() {
                let q = state.get_var(BStr::new(b"?"));
                q.as_ref().and_then(|v| parse_int::<i32>(v.as_bytes())).unwrap_or(0)
            } else {
                parse_int::<i32>(args[0].as_bytes()).unwrap_or(0)
            };
            Ok(EvalOutcome::Exit(code))
        }
        b"return" => {
            let code = if args.is_empty() {
                let q = state.get_var(BStr::new(b"?"));
                q.as_ref().and_then(|v| parse_int::<i32>(v.as_bytes())).unwrap_or(0)
            } else {
                parse_int::<i32>(args[0].as_bytes()).unwrap_or(0)
            };
            Ok(EvalOutcome::Return(code))
        }
        b"break" => {
            let n =
                if args.is_empty() { 1 } else { parse_int::<u32>(args[0].as_bytes()).unwrap_or(1) };
            Ok(EvalOutcome::Break(n))
        }
        b"continue" => {
            let n =
                if args.is_empty() { 1 } else { parse_int::<u32>(args[0].as_bytes()).unwrap_or(1) };
            Ok(EvalOutcome::Continue(n))
        }
        _ => Err(format!("{}: builtin not found", name)),
    }
}
