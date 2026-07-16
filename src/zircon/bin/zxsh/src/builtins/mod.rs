// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::eval::{EvalOutcome, ExecutionContext, ShellState};
use bstr::{BStr, BString};

pub fn is_builtin(_name: &BStr) -> bool {
    false
}

pub fn run_builtin(
    _name: &BStr,
    _args: &[BString],
    _state: &mut ShellState,
    _ctx: &mut ExecutionContext,
) -> Result<EvalOutcome, String> {
    Err("Builtins not yet implemented".to_string())
}
