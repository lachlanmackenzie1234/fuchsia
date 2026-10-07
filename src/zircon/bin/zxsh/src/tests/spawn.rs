// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::eval::testing::{SpawnedProcess, spawn_command_with_redirection};
use crate::eval::{
    EXIT_FAILURE, EXIT_NOT_FOUND, EvalOutcome, ExecutionContext, ShellState, eval_pipeline,
    eval_string,
};
use crate::fd::Fd;
use crate::parser::ast::{ASTBuilder, CommandTag, RedirectTag, RedirectTemplate, ResolvedWordPart};
use crate::process::{make_pipe, read_fd_to_end};
use bstr::{BStr, BString, ByteSlice};

#[test]
fn test_spawn_command_empty_args_error() {
    let mut state = ShellState::new();
    let ctx = ExecutionContext::initial().unwrap();

    let mut builder = ASTBuilder::new();
    let cmd_ptr = builder.add_simple_command(&[]);

    let res =
        spawn_command_with_redirection(&mut builder, cmd_ptr, &mut state, &ctx, None, None, None);
    assert!(res.is_err());
    assert_eq!(res.unwrap_err(), "No command specified");
}

#[test]
fn test_spawn_command_with_redirect_nesting() {
    let mut state = ShellState::new();
    let ctx = ExecutionContext::initial().unwrap();

    let mut builder = ASTBuilder::new();
    let simple_cmd = builder.add_simple_command(&[]);

    let dev_null =
        builder.add_resolved_word(&[ResolvedWordPart::Literal(BString::from("/dev/null"))]);
    let template = RedirectTemplate {
        tag: RedirectTag::TO_FILE,
        append: 0,
        clobber: 1,
        expand: 0,
        src_fd: Fd(1),
        dest_fd: Fd(0),
        filename: Some(dev_null),
        body: None,
    };
    let redirects_slice = builder.add_redirects_from_templates(&[template]);
    let redirect_cmd = builder.add_redirect_command(simple_cmd, redirects_slice);

    let res = spawn_command_with_redirection(
        &mut builder,
        redirect_cmd,
        &mut state,
        &ctx,
        None,
        None,
        None,
    );
    assert!(res.is_err());
    assert_eq!(res.unwrap_err(), "No command specified");
}

#[test]
fn test_eval_pipeline_simple() {
    let mut state = ShellState::new();
    let mut ctx = ExecutionContext::initial().unwrap();

    let mut builder = ASTBuilder::new();
    let cmd1 = builder.add_simple_command(&[]);
    let cmd2 = builder.add_simple_command(&[]);
    let (pipe_cmd_mut, pipe_cmd_ptr) = builder.add_command_uninit(CommandTag::PIPELINE);
    pipe_cmd_mut.left = cmd1;
    pipe_cmd_mut.right = cmd2;

    let res = eval_pipeline(&mut builder, pipe_cmd_ptr, &mut state, &mut ctx);
    assert!(res.is_err());
}

#[test]
fn test_spawn_command_not_found_returns_failed_exit_code() {
    let mut state = ShellState::new();
    let ctx = ExecutionContext::initial().unwrap();

    let mut builder = ASTBuilder::new();
    let cmd_word = builder.add_resolved_word(&[ResolvedWordPart::Literal(BString::from(
        "/nonexistent_zxsh_test_cmd",
    ))]);
    let cmd_ptr = builder.add_simple_command(&[cmd_word]);

    let (err_read, err_write) = make_pipe().unwrap();
    let res = spawn_command_with_redirection(
        &mut builder,
        cmd_ptr,
        &mut state,
        &ctx,
        None,
        None,
        Some(&err_write),
    )
    .unwrap();
    drop(err_write);

    match res {
        SpawnedProcess::Failed(code) => assert_eq!(code, EXIT_NOT_FOUND),
        SpawnedProcess::Running(_) => panic!("Expected SpawnedProcess::Failed(127)"),
    }

    let err_out = String::from_utf8(read_fd_to_end(err_read).unwrap()).unwrap();
    assert!(
        err_out.contains("zxsh: /nonexistent_zxsh_test_cmd:"),
        "expected diagnostic on stderr, got: {}",
        err_out
    );
}

#[test]
fn test_eval_pipeline_last_stage_exit_status() {
    let mut state = ShellState::new();
    let mut ctx = ExecutionContext::initial().unwrap();

    // Huge variable value causes spawn_command to fail with OUT_OF_RANGE/INVALID_ARGS -> EXIT_FAILURE (1).
    let huge = "x".repeat(100_000);
    state.set_var(BStr::new("HUGE_ARG"), BStr::new(&huge));

    // Stage 1 fails with 1, Stage 2 fails with 127 -> Pipeline exit status is 127 (last stage).
    let res = eval_string(
        b"/pkg/bin/zxsh \"$HUGE_ARG\" | /nonexistent_zxsh_stage2".as_bstr(),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(res, EvalOutcome::Code(EXIT_NOT_FOUND));
    assert_eq!(state.get_var(BStr::new("?")), Some(BString::from("127")));

    // Stage 1 fails with 127, Stage 2 fails with 1 -> Pipeline exit status is 1 (last stage).
    let res = eval_string(
        b"/nonexistent_zxsh_stage1 | /pkg/bin/zxsh \"$HUGE_ARG\"".as_bstr(),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(res, EvalOutcome::Code(EXIT_FAILURE));
    assert_eq!(state.get_var(BStr::new("?")), Some(BString::from("1")));

    // With `set -e` (`opt_errexit = true`) and `ignore_err_depth == 0`:
    // - `: | /nonexistent_zxsh_cmd` returns `Ok(EvalOutcome::Exit(127))` and sets `$?` to `127`.
    // - `/nonexistent_zxsh_cmd | true` returns `Ok(EvalOutcome::Code(0))` and sets `$?` to `0`.
    state.opt_errexit = true;
    let res = eval_string(b": | /nonexistent_zxsh_cmd".as_bstr(), &mut state, &mut ctx).unwrap();
    assert_eq!(res, EvalOutcome::Exit(EXIT_NOT_FOUND));
    assert_eq!(state.get_var(BStr::new("?")), Some(BString::from("127")));

    let res = eval_string(b"/nonexistent_zxsh_cmd | true".as_bstr(), &mut state, &mut ctx).unwrap();
    assert_eq!(res, EvalOutcome::Code(0));
    assert_eq!(state.get_var(BStr::new("?")), Some(BString::from("0")));
}
