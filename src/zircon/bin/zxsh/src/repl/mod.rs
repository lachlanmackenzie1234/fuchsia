// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::eval::{
    EvalOutcome, ExecutionContext, ShellState, eval_command, expand_prompt, run_exit_trap,
};
use crate::parser::ast::ASTBuilder;
use crate::parser::{ParseError, parse_script, tokenize};
use crate::tty::ShellSignals;
use bstr::{BStr, BString, ByteSlice, ByteVec};

mod completion;
mod linenoise;
use std::io::{BufRead, IsTerminal};

const DEFAULT_PS1: &str = "$ ";
const DEFAULT_PS2: &str = "> ";

fn get_prompt(input_buffer: &BStr, state: &mut ShellState, ctx: &ExecutionContext) -> BString {
    let prompt_var =
        if input_buffer.is_empty() { bstr::BStr::new(b"PS1") } else { bstr::BStr::new(b"PS2") };
    let default_prompt =
        if input_buffer.is_empty() { BStr::new(DEFAULT_PS1) } else { BStr::new(DEFAULT_PS2) };
    expand_prompt(prompt_var, default_prompt, state, ctx)
}

/// Starts the interactive read-eval-print loop (REPL) using `linenoise` for command history and
/// autocompletion.
pub fn run_repl(state: ShellState) {
    let stdin = std::io::stdin();
    let is_tty = stdin.is_terminal();
    if let Some(code) = run_repl_reader(stdin.lock(), state, is_tty) {
        std::process::exit(code);
    }
}

pub fn run_repl_reader<R: BufRead>(
    mut reader: R,
    mut state: ShellState,
    is_tty: bool,
) -> Option<i32> {
    state.opt_interactive = true;
    let mut ctx = match ExecutionContext::initial() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Context error: {}", e);
            return Some(1);
        }
    };

    if is_tty {
        linenoise::history_set_max_len(100);
        linenoise::set_completion_callback(completion::tab_complete);
    }

    let mut input_buffer = BString::default();
    let mut numeof = 0;

    'repl_loop: loop {
        let prompt = get_prompt(input_buffer.as_ref(), &mut state, &ctx);

        ctx.signal_state.clear(ShellSignals::INT);

        let line = if is_tty {
            let rust_line = {
                let _scoped_env = completion::ScopedState::new(&state);
                linenoise::readline(prompt.as_ref())
            };
            let rust_line = match rust_line {
                Some(l) => {
                    numeof = 0;
                    l
                }
                None => {
                    if ctx.signal_state.is_pending(ShellSignals::INT) {
                        ctx.signal_state.clear(ShellSignals::INT);
                        println!();
                        state.set_last_status(130);
                        input_buffer.clear();
                        continue;
                    }
                    if state.opt_ignoreeof && numeof < 10 {
                        numeof += 1;
                        println!("Use \"exit\" to leave shell.");
                        continue;
                    }
                    break 'repl_loop;
                }
            };

            linenoise::history_add(rust_line.as_ref());

            let mut line = rust_line;
            line.push_byte(b'\n');
            line
        } else {
            let mut l = Vec::new();
            match reader.read_until(b'\n', &mut l) {
                Ok(0) => break 'repl_loop,
                Ok(_) => BString::from(l),
                Err(e) => {
                    eprintln!("Read line error: {}", e);
                    break 'repl_loop;
                }
            }
        };

        let sigint = ctx.signal_state.is_pending(ShellSignals::INT);
        ctx.signal_state.clear(ShellSignals::INT);
        if sigint || line.as_bytes().contains(&b'\x03') {
            println!();
            state.set_last_status(130);
            input_buffer.clear();
            continue;
        }

        input_buffer.extend_from_slice(&line);
        let trimmed = input_buffer.trim_ascii();
        if trimmed.is_empty() {
            input_buffer.clear();
            continue;
        }
        let mut builder = ASTBuilder::new();
        let tokens = match tokenize(input_buffer.as_bytes()) {
            Ok(t) => t,
            Err(ParseError::Incomplete(_)) => {
                continue;
            }
            Err(ParseError::Syntax(e)) => {
                eprintln!("Tokenize error: {}", e);
                state.set_last_status(2);
                input_buffer.clear();
                continue;
            }
        };
        let cmds = match parse_script(&mut builder, &tokens) {
            Ok(c) => c,
            Err(ParseError::Incomplete(_)) => {
                continue;
            }
            Err(ParseError::Syntax(e)) => {
                eprintln!("Parse error: {}", e);
                state.set_last_status(2);
                input_buffer.clear();
                continue;
            }
        };
        input_buffer.clear();
        for &cmd_offset in &cmds {
            match eval_command(&mut builder, cmd_offset, &mut state, &mut ctx) {
                Ok(EvalOutcome::Code(c)) => {
                    state.set_last_status(c);
                }
                Ok(EvalOutcome::Exit(c)) => {
                    run_exit_trap(&mut state, &mut ctx);
                    return Some(c);
                }
                Ok(EvalOutcome::Return(c)) => {
                    run_exit_trap(&mut state, &mut ctx);
                    return Some(c);
                }
                Ok(EvalOutcome::Break(_)) => {
                    eprintln!("zxsh: break: can only break from a loop");
                    state.set_last_status(2);
                }
                Ok(EvalOutcome::Continue(_)) => {
                    eprintln!("zxsh: continue: can only continue from a loop");
                    state.set_last_status(2);
                }
                Err(e) => {
                    if !e.is_empty() {
                        eprintln!("Eval error: {}", e);
                    }
                    state.set_last_status(1);
                }
            }
        }
    }
    run_exit_trap(&mut state, &mut ctx);
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_get_prompt_defaults() {
        let mut state = ShellState::new();
        let ctx = ExecutionContext::initial().unwrap();

        assert_eq!(get_prompt(BStr::new(""), &mut state, &ctx), BStr::new(DEFAULT_PS1));
        assert_eq!(get_prompt(BStr::new("input"), &mut state, &ctx), BStr::new(DEFAULT_PS2));
    }

    #[test]
    fn test_get_prompt_custom_vars() {
        let mut state = ShellState::new();
        state.set_var("PS1", "MYPROMPT> ");
        state.set_var("PS2", "CONTINUE> ");
        let ctx = ExecutionContext::initial().unwrap();

        assert_eq!(get_prompt(BStr::new(""), &mut state, &ctx), BStr::new("MYPROMPT> "));
        assert_eq!(get_prompt(BStr::new("line1"), &mut state, &ctx), BStr::new("CONTINUE> "));
    }

    #[test]
    fn test_get_prompt_expansion_error() {
        let mut state = ShellState::new();
        state.set_var("PS1", "$(( 1 / 0 ))");
        let ctx = ExecutionContext::initial().unwrap();

        // Expands to error -> falls back to default prompt DEFAULT_PS1
        assert_eq!(get_prompt(BStr::new(""), &mut state, &ctx), BStr::new(DEFAULT_PS1));
    }

    #[test]
    fn test_run_repl_reader_execution() {
        let input = "x=10\nexport Y=$x\n\n  \nbreak\ncontinue\nexit 5\n";
        let cursor = Cursor::new(input);
        let res = run_repl_reader(cursor, ShellState::new(), false);
        assert_eq!(res, Some(5));
    }

    #[test]
    fn test_run_repl_reader_incomplete_and_errors() {
        let input = "export Y='multi\nline'\nif; then\n'unterminated quote\n";
        let cursor = Cursor::new(input);
        let res = run_repl_reader(cursor, ShellState::new(), false);
        assert_eq!(res, None);
    }
}
