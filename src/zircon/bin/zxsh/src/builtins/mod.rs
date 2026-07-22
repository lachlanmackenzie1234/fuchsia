// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::eval::{
    ClosedReader, ClosedWriter, EXIT_FAILURE, EXIT_SUCCESS, EvalOutcome, ExecutionContext,
    ShellState, VarName,
};
use bstr::{BString, ByteSlice};

pub mod essential;

#[derive(Clone, Copy)]
enum BuiltinType {
    Dot,
    Colon,
    Alias,
    Break,
    Cd,
    Chdir,
    Command,
    Continue,
    Eval,
    Exec,
    Exit,
    Export,
    False,
    Getopts,
    Hash,
    Local,
    Read,
    Readonly,
    Return,
    Set,
    Shift,
    Trap,
    True,
    Type,
    Ulimit,
    Umask,
    Unalias,
    Unset,
    Wait,
}

struct BuiltinEntry {
    name: [u8; 18],
    len: u8,
    func: BuiltinType,
}

const fn make_entry(name: &[u8], func: BuiltinType) -> BuiltinEntry {
    let mut name_arr = [0u8; 18];
    let len = name.len();
    if len > 18 {
        panic!("Builtin name too long");
    }
    let mut i = 0;
    while i < len {
        name_arr[i] = name[i];
        i += 1;
    }
    BuiltinEntry { name: name_arr, len: len as u8, func }
}

static BUILTINS: &[BuiltinEntry] = &[
    make_entry(b".", BuiltinType::Dot),
    make_entry(b":", BuiltinType::Colon),
    make_entry(b"alias", BuiltinType::Alias),
    make_entry(b"break", BuiltinType::Break),
    make_entry(b"cd", BuiltinType::Cd),
    make_entry(b"chdir", BuiltinType::Chdir),
    make_entry(b"command", BuiltinType::Command),
    make_entry(b"continue", BuiltinType::Continue),
    make_entry(b"eval", BuiltinType::Eval),
    make_entry(b"exec", BuiltinType::Exec),
    make_entry(b"exit", BuiltinType::Exit),
    make_entry(b"export", BuiltinType::Export),
    make_entry(b"false", BuiltinType::False),
    make_entry(b"getopts", BuiltinType::Getopts),
    make_entry(b"hash", BuiltinType::Hash),
    make_entry(b"local", BuiltinType::Local),
    make_entry(b"read", BuiltinType::Read),
    make_entry(b"readonly", BuiltinType::Readonly),
    make_entry(b"return", BuiltinType::Return),
    make_entry(b"set", BuiltinType::Set),
    make_entry(b"shift", BuiltinType::Shift),
    make_entry(b"trap", BuiltinType::Trap),
    make_entry(b"true", BuiltinType::True),
    make_entry(b"type", BuiltinType::Type),
    make_entry(b"ulimit", BuiltinType::Ulimit),
    make_entry(b"umask", BuiltinType::Umask),
    make_entry(b"unalias", BuiltinType::Unalias),
    make_entry(b"unset", BuiltinType::Unset),
    make_entry(b"wait", BuiltinType::Wait),
];

/// Checks if the given command name corresponds to an internal shell builtin.
pub fn is_builtin(name: impl VarName) -> bool {
    let name = name.to_bstr();
    BUILTINS
        .binary_search_by_key(&name.as_bytes(), |entry| &entry.name[..entry.len as usize])
        .is_ok()
}

use std::io::{Read, Write};

fn with_io_streams<R>(
    ctx: &mut ExecutionContext,
    f: impl FnOnce(&mut dyn Read, &mut dyn Write, &mut dyn Write) -> R,
) -> R {
    let mut default_in = ClosedReader;
    let mut default_out = ClosedWriter;
    let mut default_err = ClosedWriter;

    let mut stdin_ref = ctx.stdin();
    let in_file: &mut dyn Read = match &mut stdin_ref {
        Some(f) => f,
        None => &mut default_in,
    };
    let mut stdout_ref = ctx.stdout();
    let out_file: &mut dyn Write = match &mut stdout_ref {
        Some(f) => f,
        None => &mut default_out,
    };
    let mut stderr_ref = ctx.stderr();
    let err_file: &mut dyn Write = match &mut stderr_ref {
        Some(f) => f,
        None => &mut default_err,
    };
    f(in_file, out_file, err_file)
}

fn with_io(
    ctx: &mut ExecutionContext,
    args: &[BString],
    state: &mut ShellState,
    f: impl FnOnce(&[BString], &mut ShellState, &mut dyn Read, &mut dyn Write, &mut dyn Write) -> i32,
) -> Result<EvalOutcome, String> {
    with_io_streams(ctx, |r, w, e| Ok(EvalOutcome::Code(f(args, state, r, w, e))))
}

/// Executes an internal shell builtin command with the provided argument list and execution
/// context.
pub fn run_builtin(
    name: impl VarName,
    args: &[BString],
    state: &mut ShellState,
    ctx: &mut ExecutionContext,
) -> Result<EvalOutcome, String> {
    let name = name.to_bstr();
    let idx = BUILTINS
        .binary_search_by_key(&name.as_bytes(), |entry| &entry.name[..entry.len as usize])
        .map_err(|_| format!("builtin not found: {}", name))?;

    match BUILTINS[idx].func {
        BuiltinType::Dot => essential::builtin_dot(args, state, ctx),
        BuiltinType::Eval => essential::builtin_eval(args, state, ctx),
        BuiltinType::Exec => essential::builtin_exec(args, state, ctx),
        BuiltinType::Command => essential::builtin_command(args, state, ctx),
        BuiltinType::Colon | BuiltinType::True => Ok(EvalOutcome::Code(EXIT_SUCCESS)),
        BuiltinType::False => Ok(EvalOutcome::Code(EXIT_FAILURE)),
        BuiltinType::Break => essential::builtin_break(args, state, ctx),
        BuiltinType::Continue => essential::builtin_continue(args, state, ctx),
        BuiltinType::Return => essential::builtin_return(args, state, ctx),
        BuiltinType::Exit => essential::builtin_exit(args, state, ctx),
        BuiltinType::Alias => with_io(ctx, args, state, essential::builtin_alias),
        BuiltinType::Cd | BuiltinType::Chdir => with_io(ctx, args, state, essential::builtin_cd),
        BuiltinType::Export => with_io(ctx, args, state, essential::builtin_export),
        BuiltinType::Getopts => with_io(ctx, args, state, essential::builtin_getopts),
        BuiltinType::Hash => with_io(ctx, args, state, essential::builtin_hash),
        BuiltinType::Local => with_io(ctx, args, state, essential::builtin_local),
        BuiltinType::Read => with_io(ctx, args, state, essential::builtin_read),
        BuiltinType::Readonly => with_io(ctx, args, state, essential::builtin_readonly),
        BuiltinType::Set => with_io(ctx, args, state, essential::builtin_set),
        BuiltinType::Shift => with_io(ctx, args, state, essential::builtin_shift),
        BuiltinType::Trap => with_io(ctx, args, state, essential::builtin_trap),
        BuiltinType::Type => with_io(ctx, args, state, essential::builtin_type),
        BuiltinType::Ulimit => with_io(ctx, args, state, essential::builtin_ulimit),
        BuiltinType::Umask => with_io(ctx, args, state, essential::builtin_umask),
        BuiltinType::Unalias => with_io(ctx, args, state, essential::builtin_unalias),
        BuiltinType::Unset => with_io(ctx, args, state, essential::builtin_unset),
        BuiltinType::Wait => with_io(ctx, args, state, essential::builtin_wait),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtins_sorted() {
        for i in 1..BUILTINS.len() {
            assert!(
                &BUILTINS[i - 1].name[..BUILTINS[i - 1].len as usize]
                    < &BUILTINS[i].name[..BUILTINS[i].len as usize],
                "BUILTINS table is not sorted: {:?} vs {:?}",
                std::str::from_utf8(&BUILTINS[i - 1].name[..BUILTINS[i - 1].len as usize]),
                std::str::from_utf8(&BUILTINS[i].name[..BUILTINS[i].len as usize])
            );
        }
    }
}
