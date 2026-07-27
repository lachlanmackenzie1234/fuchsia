// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::errors::zx_status_str;
use crate::eval::{
    ClosedWriter, EXIT_CANNOT_EXEC, EXIT_FAILURE, EXIT_NOT_FOUND, EXIT_SUCCESS, EXIT_SYNTAX_ERROR,
    EvalOutcome, ExecutionContext, RLIM_INFINITY, RLIMIT_CORE, RLIMIT_FSIZE, RLIMIT_NOFILE,
    ShellState, clone_fd_to_action, eval_string, wait_for_process_to_exit,
};
use crate::fd::Fd;
use crate::process::spawn_command;
use crate::string::{LineChar, parse_int, parse_mode_mask, split_ifs_read, split_key_value};
use bstr::{BStr, BString, ByteSlice};
use std::io::{Read, Write};

use super::{is_builtin, run_builtin};

macro_rules! write_out {
    ($ctx:expr, $($arg:tt)*) => {{
        if let Some(mut file) = $ctx.stdout() {
            let _ = writeln!(file, $($arg)*);
        }
    }};
}

macro_rules! write_err {
    ($ctx:expr, $($arg:tt)*) => {{
        if let Some(mut file) = $ctx.stderr() {
            let _ = writeln!(file, $($arg)*);
        }
    }};
}

pub fn builtin_cd(
    args: &[BString],
    state: &mut ShellState,
    _stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    let prev_cwd = state.cwd().to_owned();

    let dest = if args.is_empty() {
        match state.get_var(b"HOME") {
            Some(home) if !home.is_empty() => home,
            _ => BString::from("/"),
        }
    } else if args[0] == "-" {
        let oldpwd = state.get_var(b"OLDPWD").unwrap_or_default();
        if oldpwd.is_empty() {
            let _ = writeln!(stderr, "cd: OLDPWD not set");
            return EXIT_FAILURE;
        }
        oldpwd
    } else {
        args[0].clone()
    };

    let path = match dest.to_path() {
        Ok(path) => path,
        Err(err) => {
            let _ = writeln!(stderr, "cd: invalid path {}: {}", dest, err);
            return EXIT_FAILURE;
        }
    };

    let target_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        match state.cwd().to_path() {
            Ok(base) => base.join(path),
            Err(_) => path.to_path_buf(),
        }
    };

    if let Err(err) = std::env::set_current_dir(&target_path) {
        let _ = writeln!(stderr, "cd: {}: {}", dest, err);
        return EXIT_FAILURE;
    }

    let new_pwd = match <[u8]>::from_path(&target_path) {
        Some(bytes) => BString::from(bytes),
        None => dest,
    };

    if !args.is_empty() && args[0] == "-" {
        let _ = writeln!(stdout, "{}", new_pwd);
    }

    if !prev_cwd.is_empty() {
        let _ = state.set_var(b"OLDPWD", &prev_cwd);
    }
    state.set_cwd(new_pwd);

    EXIT_SUCCESS
}

fn parse_status_code(args: &[BString], state: &ShellState) -> Option<i32> {
    if args.is_empty() {
        let q_var = state.get_var(b"?");
        Some(q_var.as_ref().and_then(|v| parse_int::<i32>(v.as_bytes())).unwrap_or(EXIT_SUCCESS))
    } else {
        parse_int::<i32>(args[0].as_bytes())
    }
}

pub fn builtin_exit(
    args: &[BString],
    state: &mut ShellState,
    ctx: &mut ExecutionContext,
) -> Result<EvalOutcome, String> {
    let code = match parse_status_code(args, state) {
        Some(code) => code,
        None => {
            write_err!(ctx, "exit: numeric argument required");
            EXIT_SUCCESS
        }
    };
    Ok(EvalOutcome::Exit(code))
}

pub fn builtin_return(
    args: &[BString],
    state: &mut ShellState,
    ctx: &mut ExecutionContext,
) -> Result<EvalOutcome, String> {
    let code = match parse_status_code(args, state) {
        Some(code) => code,
        None => {
            write_err!(ctx, "return: illegal number: {}", args[0]);
            return Ok(EvalOutcome::Code(EXIT_SYNTAX_ERROR));
        }
    };
    Ok(EvalOutcome::Return(code))
}

pub fn builtin_export(
    args: &[BString],
    state: &mut ShellState,
    _stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    if args.is_empty() || args[0] == "-p" {
        for (name, val) in state.vars().sorted() {
            let _ = writeln!(stdout, "export {}='{}'", name, val);
        }
    } else {
        for arg in args {
            if let Some((name, val)) = split_key_value(arg.as_bytes()) {
                if state.is_readonly(name) {
                    let _ = writeln!(stderr, "export: {}: readonly variable", name);
                    return EXIT_FAILURE;
                }
                state.set_var(name, val);
                state.export_var(name);
            } else {
                state.export_var(arg);
            }
        }
    }
    EXIT_SUCCESS
}

pub fn builtin_unset(
    args: &[BString],
    state: &mut ShellState,
    _stdin: &mut dyn Read,
    _stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    enum UnsetMode {
        Default,
        Function,
        Variable,
    }

    let mut mode = UnsetMode::Default;
    let mut vars = args;
    if !args.is_empty() {
        if args[0] == "-f" {
            mode = UnsetMode::Function;
            vars = &args[1..];
        } else if args[0] == "-v" {
            mode = UnsetMode::Variable;
            vars = &args[1..];
        }
    }
    for arg in vars {
        match mode {
            UnsetMode::Function => {
                state.remove_function(arg);
            }
            UnsetMode::Variable => {
                if state.is_readonly(arg) {
                    let _ = writeln!(stderr, "unset: {}: readonly variable", arg);
                    return EXIT_FAILURE;
                }
                state.unset_var(arg);
            }
            UnsetMode::Default => {
                if state.is_readonly(arg) {
                    let _ = writeln!(stderr, "unset: {}: readonly variable", arg);
                    return EXIT_FAILURE;
                }
                if state.get_var(arg).is_some() {
                    state.unset_var(arg);
                } else {
                    state.remove_function(arg);
                }
            }
        }
    }
    EXIT_SUCCESS
}

pub fn builtin_local(
    args: &[BString],
    state: &mut ShellState,
    _stdin: &mut dyn Read,
    _stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    if !state.is_in_function() {
        let _ = writeln!(stderr, "local: can only be used in a function");
        return EXIT_FAILURE;
    }
    for arg in args {
        if let Some((name, val)) = split_key_value(arg.as_bytes()) {
            if state.is_readonly(name) {
                let _ = writeln!(stderr, "local: {}: readonly variable", name);
                return EXIT_FAILURE;
            }
            state.declare_local(name, Some(val));
        } else {
            state.declare_local(arg.as_ref(), None);
        }
    }
    EXIT_SUCCESS
}

pub fn builtin_set(
    args: &[BString],
    state: &mut ShellState,
    _stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    if args.is_empty() {
        for (name, val) in state.all_vars().sorted_entries() {
            let _ = writeln!(stdout, "{}='{}'", name, val);
        }
    } else {
        let mut arg_idx = 0;
        let mut set_positional = false;
        let mut new_args = Vec::new();

        while arg_idx < args.len() {
            let arg = &args[arg_idx];
            if arg == "--" {
                set_positional = true;
                new_args.extend(args[arg_idx + 1..].to_vec());
                break;
            } else if (arg.starts_with(b"-") || arg.starts_with(b"+")) && arg.len() > 1 {
                let enable = arg.starts_with(b"-");
                let prefix = if enable { "-" } else { "+" };
                for &flag_char in arg.as_bytes().iter().skip(1) {
                    if state.set_option_by_flag(flag_char, enable).is_err() {
                        let _ = writeln!(
                            stderr,
                            "set: unknown option: {}{}",
                            prefix, flag_char as char
                        );
                        return EXIT_FAILURE;
                    }
                }
                arg_idx += 1;
            } else {
                set_positional = true;
                new_args.extend(args[arg_idx..].to_vec());
                break;
            }
        }

        if set_positional {
            state.set_args(new_args);
        }
    }
    EXIT_SUCCESS
}

pub fn builtin_shift(
    args: &[BString],
    state: &mut ShellState,
    _stdin: &mut dyn Read,
    _stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    let shift_count = if args.is_empty() {
        1
    } else {
        match parse_int::<usize>(args[0].as_bytes()) {
            Some(val) => val,
            None => {
                let _ = writeln!(stderr, "shift: invalid number");
                return EXIT_FAILURE;
            }
        }
    };
    let current_args = state.get_args();
    if shift_count > current_args.len() {
        let _ = writeln!(stderr, "shift: shift count out of range");
        return EXIT_FAILURE;
    }
    let mut new_args = current_args;
    new_args.drain(0..shift_count);
    state.set_args(new_args);
    EXIT_SUCCESS
}

fn is_valid_signal(sig: &BStr) -> bool {
    let bytes = sig.as_bytes();
    let upper = bytes.to_ascii_uppercase();
    match upper.as_slice() {
        b"0" | b"EXIT" | b"SIGEXIT" | b"1" | b"HUP" | b"SIGHUP" | b"2" | b"INT" | b"SIGINT"
        | b"3" | b"QUIT" | b"SIGQUIT" | b"6" | b"ABRT" | b"SIGABRT" | b"9" | b"KILL"
        | b"SIGKILL" | b"14" | b"ALRM" | b"SIGALRM" | b"15" | b"TERM" | b"SIGTERM" => true,
        _ => {
            if upper.iter().all(|c| c.is_ascii_digit()) {
                true
            } else if let Some(stripped) = upper.strip_prefix(b"SIG") {
                !stripped.is_empty() && stripped.iter().all(|c| c.is_ascii_alphabetic())
            } else {
                !upper.is_empty() && upper.iter().all(|c| c.is_ascii_alphabetic())
            }
        }
    }
}

fn normalize_signal(sig: &BStr) -> BString {
    let bytes = sig.as_bytes();
    let upper = bytes.to_ascii_uppercase();
    match upper.as_slice() {
        b"0" | b"EXIT" | b"SIGEXIT" => BString::from("EXIT"),
        b"1" | b"HUP" | b"SIGHUP" => BString::from("HUP"),
        b"2" | b"INT" | b"SIGINT" => BString::from("INT"),
        b"3" | b"QUIT" | b"SIGQUIT" => BString::from("QUIT"),
        b"6" | b"ABRT" | b"SIGABRT" => BString::from("ABRT"),
        b"9" | b"KILL" | b"SIGKILL" => BString::from("KILL"),
        b"14" | b"ALRM" | b"SIGALRM" => BString::from("ALRM"),
        b"15" | b"TERM" | b"SIGTERM" => BString::from("TERM"),
        _ => {
            if let Some(stripped) = upper.strip_prefix(b"SIG") {
                BString::from(stripped)
            } else {
                BString::from(upper)
            }
        }
    }
}

pub fn builtin_trap(
    args: &[BString],
    state: &mut ShellState,
    _stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    if args.is_empty() {
        for (sig_name, action) in state.traps.sorted_entries() {
            let _ = writeln!(stdout, "trap -- '{}' {}", action, sig_name);
        }
    } else if args.len() == 1 {
        let sig = &args[0];
        if is_valid_signal(sig.as_bstr()) {
            let normalized_sig = normalize_signal(sig.as_bstr());
            state.traps.remove(normalized_sig.as_bstr());
        } else {
            let _ = writeln!(stderr, "trap: missing signal specs");
            return EXIT_FAILURE;
        }
    } else {
        let action = &args[0];
        let sigs = &args[1..];
        for sig in sigs {
            if is_valid_signal(sig.as_bstr()) {
                let normalized_sig = normalize_signal(sig.as_bstr());
                if action == "-" {
                    state.traps.remove(normalized_sig.as_bstr());
                } else {
                    state.traps.insert(normalized_sig, action.clone());
                }
            } else {
                let _ = writeln!(stderr, "trap: invalid signal: {}", sig);
                return EXIT_FAILURE;
            }
        }
    }
    EXIT_SUCCESS
}

pub fn builtin_read(
    args: &[BString],
    state: &mut ShellState,
    stdin: &mut dyn Read,
    _stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    let mut raw = false;
    let mut vars = args;
    if !vars.is_empty() && vars[0] == "-r" {
        raw = true;
        vars = &vars[1..];
    }
    let temp_vars;
    if vars.is_empty() {
        temp_vars = vec![BString::from("REPLY")];
        vars = &temp_vars;
    }

    let mut line = Vec::new();
    let mut buf = [0; 1];
    loop {
        let n = match stdin.read(&mut buf) {
            Ok(n) => n,
            Err(err) => {
                let _ = writeln!(stderr, "read: {}", err);
                return EXIT_FAILURE;
            }
        };
        if n == 0 {
            break;
        }
        let c = buf[0];
        if c == b'\n' {
            break;
        }
        if !raw && c == b'\\' {
            let mut next_buf = [0; 1];
            let next_n = match stdin.read(&mut next_buf) {
                Ok(n) => n,
                Err(err) => {
                    let _ = writeln!(stderr, "read: {}", err);
                    return EXIT_FAILURE;
                }
            };
            if next_n > 0 {
                let next_c = next_buf[0];
                if next_c != b'\n' {
                    line.push(next_c);
                }
                continue;
            }
        }
        line.push(c);
    }

    if line.is_empty() && buf[0] == 0 {
        return EXIT_FAILURE;
    }

    let ifs = state.get_var(b"IFS").unwrap_or_else(|| BString::from(" \t\n"));
    let line_chars: Vec<LineChar> =
        line.iter().map(|&byte| LineChar { byte, escaped: false }).collect();
    let fields = split_ifs_read(&line_chars, ifs.as_ref(), vars.len());

    for (i, var) in vars.iter().enumerate() {
        if state.is_readonly(var) {
            let _ = writeln!(stderr, "read: {}: readonly variable", var);
            return EXIT_FAILURE;
        }
        let val = fields.get(i).cloned().unwrap_or_default();
        state.set_var(var, &val);
    }
    EXIT_SUCCESS
}

pub fn builtin_eval(
    args: &[BString],
    state: &mut ShellState,
    ctx: &mut ExecutionContext,
) -> Result<EvalOutcome, String> {
    let mut cmd_bytes = Vec::new();
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            cmd_bytes.push(b' ');
        }
        cmd_bytes.extend_from_slice(arg.as_bytes());
    }
    let cmd_str = BString::from(cmd_bytes);
    eval_string(cmd_str.as_ref(), state, ctx)
}

fn execute_external_process(
    caller_name: &str,
    args: &[BString],
    state: &mut ShellState,
    ctx: &mut ExecutionContext,
) -> EvalOutcome {
    let mut actions = Vec::new();
    for (fd_opt, target) in
        [(ctx.stdin(), Fd::STDIN), (ctx.stdout(), Fd::STDOUT), (ctx.stderr(), Fd::STDERR)]
    {
        if let Some(fd) = fd_opt {
            if let Some(action) = clone_fd_to_action(fd, target) {
                actions.push(action);
            }
        }
    }
    let proc = match spawn_command(args, &state.vars(), &mut actions) {
        Ok(proc) => proc,
        Err(status) => {
            if let Some(mut err) = ctx.stderr() {
                let _ = writeln!(
                    err,
                    "{}: failed to spawn {}: {}",
                    caller_name,
                    args[0],
                    zx_status_str(status)
                );
            }
            let code = if status == zx::Status::NOT_FOUND {
                EXIT_NOT_FOUND
            } else if status == zx::Status::ACCESS_DENIED {
                EXIT_CANNOT_EXEC
            } else {
                EXIT_FAILURE
            };
            return EvalOutcome::Code(code);
        }
    };
    match wait_for_process_to_exit(&proc, ctx) {
        Ok(code) => EvalOutcome::Code(code),
        Err(e) => {
            if let Some(mut err) = ctx.stderr() {
                let _ = writeln!(err, "{}: wait failed: {}", caller_name, e);
            }
            EvalOutcome::Code(EXIT_FAILURE)
        }
    }
}

pub fn builtin_exec(
    args: &[BString],
    state: &mut ShellState,
    ctx: &mut ExecutionContext,
) -> Result<EvalOutcome, String> {
    if args.is_empty() {
        Ok(EvalOutcome::Code(EXIT_SUCCESS))
    } else {
        match execute_external_process("exec", args, state, ctx) {
            EvalOutcome::Code(code)
                if code != EXIT_NOT_FOUND && code != EXIT_CANNOT_EXEC && code != EXIT_FAILURE =>
            {
                Ok(EvalOutcome::Exit(code))
            }
            outcome => Ok(outcome),
        }
    }
}

pub fn builtin_type(
    args: &[BString],
    state: &mut ShellState,
    _stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    _stderr: &mut dyn Write,
) -> i32 {
    let mut exit_code = EXIT_SUCCESS;
    for name in args {
        if state.get_function(name).is_some() {
            let _ = writeln!(stdout, "{} is a function", name);
        } else if is_builtin(name.as_bstr()) {
            let _ = writeln!(stdout, "{} is a shell builtin", name);
        } else {
            if let Some(resolved) = state.path().resolve(name.as_ref()) {
                let _ = writeln!(stdout, "{} is {}", name, resolved);
            } else {
                let _ = writeln!(stdout, "{}: not found", name);
                exit_code = EXIT_NOT_FOUND;
            }
        }
    }
    exit_code
}

pub fn builtin_wait(
    args: &[BString],
    state: &mut ShellState,
    _stdin: &mut dyn Read,
    _stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    let mut jobs_to_wait = Vec::new();
    if args.is_empty() {
        jobs_to_wait.append(&mut state.bg_jobs);
    } else {
        let mut clean_args = args;
        if !clean_args.is_empty() && clean_args[0] == "--" {
            clean_args = &clean_args[1..];
        }
        if clean_args.is_empty() {
            jobs_to_wait.append(&mut state.bg_jobs);
        } else {
            let mut koids = Vec::new();
            for arg in clean_args {
                let Some(koid) = parse_int::<u64>(arg.as_bytes()) else {
                    let _ = writeln!(stderr, "wait: invalid process ID: {}", arg);
                    return EXIT_FAILURE;
                };
                koids.push(koid);
            }
            let mut found_any = false;
            let mut remaining_jobs = Vec::new();
            for job in state.bg_jobs.drain(..) {
                if let Ok(koid) = job.process.koid() {
                    if koids.contains(&koid.raw_koid()) {
                        jobs_to_wait.push(job);
                        found_any = true;
                    } else {
                        remaining_jobs.push(job);
                    }
                } else {
                    remaining_jobs.push(job);
                }
            }
            state.bg_jobs = remaining_jobs;
            if !found_any {
                return EXIT_NOT_FOUND;
            }
        }
    }

    let mut exit_code = EXIT_SUCCESS;
    for job in jobs_to_wait {
        if job
            .process
            .wait_one(zx::Signals::PROCESS_TERMINATED, zx::MonotonicInstant::INFINITE)
            .to_result()
            .is_ok()
        {
            if let Ok(info) = job.process.info() {
                exit_code = info.return_code as i32;
            }
        }
    }
    exit_code
}

pub fn builtin_dot(
    args: &[BString],
    state: &mut ShellState,
    ctx: &mut ExecutionContext,
) -> Result<EvalOutcome, String> {
    if args.is_empty() {
        write_err!(ctx, ".: missing script operand");
        return Ok(EvalOutcome::Code(EXIT_SYNTAX_ERROR));
    }
    let script_path = &args[0];
    let script_args = args[1..].to_vec();

    let resolved_path = state.path().resolve(script_path.as_ref()).or_else(|| {
        if script_path.find(b"/").is_some() { Some(script_path.clone()) } else { None }
    });

    let resolved_path = match resolved_path {
        Some(path) => path,
        None => {
            write_err!(ctx, ".: {}: script not found", script_path);
            return Ok(EvalOutcome::Code(EXIT_NOT_FOUND));
        }
    };

    let path = match resolved_path.to_path() {
        Ok(path) => path,
        Err(err) => {
            write_err!(ctx, ".: invalid path {}: {}", resolved_path, err);
            return Ok(EvalOutcome::Code(EXIT_NOT_FOUND));
        }
    };
    let content = match std::fs::read(path) {
        Ok(content) => content,
        Err(err) => {
            write_err!(ctx, ".: failed to read {}: {}", resolved_path, err);
            let code = if err.kind() == std::io::ErrorKind::NotFound {
                EXIT_NOT_FOUND
            } else {
                EXIT_FAILURE
            };
            return Ok(EvalOutcome::Code(code));
        }
    };

    let mut old_args = None;
    if !script_args.is_empty() {
        old_args = Some(state.get_args());
        state.set_args(script_args);
    }

    let res = eval_string(BStr::new(&content), state, ctx);

    if let Some(args) = old_args {
        state.set_args(args);
    }

    match res {
        Ok(EvalOutcome::Return(code)) => Ok(EvalOutcome::Code(code)),
        other => other,
    }
}

pub fn builtin_getopts(
    args: &[BString],
    state: &mut ShellState,
    _stdin: &mut dyn Read,
    _stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    if args.len() < 2 {
        let _ = writeln!(stderr, "getopts: usage: getopts optstring name [arg...]");
        return EXIT_SYNTAX_ERROR;
    }
    let optstring = &args[0];
    let name_var = &args[1];

    let clean_args = if args.len() > 2 { args[2..].to_vec() } else { state.get_args() };

    let optind_str = state.get_var(b"OPTIND").unwrap_or_else(|| BString::from("1"));
    let mut optind = parse_int::<usize>(optind_str.as_bytes()).unwrap_or(1);

    if optind == 0 {
        optind = 1;
    }

    if optind > clean_args.len() {
        state.set_var(name_var, b"?");
        return EXIT_FAILURE;
    }

    let arg = &clean_args[optind - 1];
    if !arg.starts_with(b"-") || arg == "-" {
        state.set_var(name_var, b"?");
        return EXIT_FAILURE;
    }

    if arg == "--" {
        let optind_next = (optind + 1).to_string();
        state.set_var(b"OPTIND", optind_next.as_bytes());
        state.set_var(name_var, b"?");
        return EXIT_FAILURE;
    }

    let offset = state.optopt_offset;

    let bytes = arg.as_bytes();
    if offset >= bytes.len() {
        state.set_var(name_var, b"?");
        return EXIT_FAILURE;
    }

    let opt_byte = bytes[offset];

    if let Some(pos) = optstring.find(&[opt_byte]) {
        let requires_arg = pos + 1 < optstring.len() && optstring.as_bytes()[pos + 1] == b':';
        if requires_arg {
            if offset + 1 < bytes.len() {
                let val = BStr::new(&bytes[offset + 1..]);
                state.set_var(b"OPTARG", val);
                let optind_next = (optind + 1).to_string();
                state.set_var(b"OPTIND", optind_next.as_bytes());
                state.optopt_offset = 1;
            } else {
                if optind < clean_args.len() {
                    let val = &clean_args[optind];
                    state.set_var(b"OPTARG", val);
                    let optind_next = (optind + 2).to_string();
                    state.set_var(b"OPTIND", optind_next.as_bytes());
                    state.optopt_offset = 1;
                } else {
                    state.set_var(b"OPTARG", b"");
                    if optstring.starts_with(b":") {
                        state.set_var(name_var, b":");
                        let opt_char_str = (opt_byte as char).to_string();
                        state.set_var(b"OPTARG", opt_char_str.as_bytes());
                    } else {
                        let _ = writeln!(
                            stderr,
                            "zxsh: getopts: option requires an argument -- {}",
                            opt_byte as char
                        );
                        state.set_var(name_var, b"?");
                    }
                    let optind_next = (optind + 1).to_string();
                    state.set_var(b"OPTIND", optind_next.as_bytes());
                    state.optopt_offset = 1;
                    return EXIT_SUCCESS;
                }
            }
            let opt_char_str = (opt_byte as char).to_string();
            state.set_var(name_var, opt_char_str.as_bytes());
        } else {
            let opt_char_str = (opt_byte as char).to_string();
            state.set_var(name_var, opt_char_str.as_bytes());
            if offset + 1 < bytes.len() {
                state.optopt_offset = offset + 1;
            } else {
                let optind_next = (optind + 1).to_string();
                state.set_var(b"OPTIND", optind_next.as_bytes());
                state.optopt_offset = 1;
            }
        }
    } else {
        if optstring.starts_with(b":") {
            state.set_var(name_var, b"?");
            let opt_char_str = (opt_byte as char).to_string();
            state.set_var(b"OPTARG", opt_char_str.as_bytes());
        } else {
            let _ = writeln!(stderr, "zxsh: getopts: illegal option -- {}", opt_byte as char);
            state.set_var(name_var, b"?");
            state.unset_var(b"OPTARG");
        }
        if offset + 1 < bytes.len() {
            state.optopt_offset = offset + 1;
        } else {
            let optind_next = (optind + 1).to_string();
            state.set_var(b"OPTIND", optind_next.as_bytes());
            state.optopt_offset = 1;
        }
    }
    EXIT_SUCCESS
}

pub fn builtin_command(
    args: &[BString],
    state: &mut ShellState,
    ctx: &mut ExecutionContext,
) -> Result<EvalOutcome, String> {
    if args.is_empty() {
        return Ok(EvalOutcome::Code(EXIT_SUCCESS));
    }
    let mode = &args[0];
    if mode == "-v" {
        if args.len() < 2 {
            return Ok(EvalOutcome::Code(EXIT_SUCCESS));
        }
        let name = &args[1];
        if state.get_function(name).is_some() || is_builtin(name.as_bstr()) {
            write_out!(ctx, "{}", name);
            return Ok(EvalOutcome::Code(EXIT_SUCCESS));
        }
        if let Some(resolved) = state.path().resolve(name.as_ref()) {
            write_out!(ctx, "{}", resolved);
            return Ok(EvalOutcome::Code(EXIT_SUCCESS));
        }
        return Ok(EvalOutcome::Code(EXIT_NOT_FOUND));
    } else if mode == "-V" {
        if args.len() < 2 {
            return Ok(EvalOutcome::Code(EXIT_SUCCESS));
        }
        let type_args = &args[1..];
        let mut default_out = ClosedWriter;
        let mut default_err = ClosedWriter;
        let mut stdout_ref = ctx.stdout();
        let stdout: &mut dyn Write = match &mut stdout_ref {
            Some(file) => file,
            None => &mut default_out,
        };
        let mut stderr_ref = ctx.stderr();
        let stderr: &mut dyn Write = match &mut stderr_ref {
            Some(file) => file,
            None => &mut default_err,
        };
        let code = builtin_type(type_args, state, &mut std::io::empty(), stdout, stderr);
        return Ok(EvalOutcome::Code(code));
    }

    // Otherwise, bypass shell function lookup for args[0]
    let cmd_name = &args[0];
    if is_builtin(cmd_name.as_bstr()) {
        return run_builtin(cmd_name.as_bstr(), &args[1..], state, ctx);
    }

    Ok(execute_external_process("command", args, state, ctx))
}

fn parse_loop_count(name: &str, args: &[BString], ctx: &mut ExecutionContext) -> Option<u32> {
    if args.is_empty() {
        Some(1)
    } else {
        match parse_int::<u32>(args[0].as_bytes()) {
            Some(count) if count > 0 => Some(count),
            _ => {
                write_err!(ctx, "{}: illegal number: {}", name, args[0]);
                None
            }
        }
    }
}

pub fn builtin_break(
    args: &[BString],
    _env: &mut ShellState,
    ctx: &mut ExecutionContext,
) -> Result<EvalOutcome, String> {
    match parse_loop_count("break", args, ctx) {
        Some(count) => Ok(EvalOutcome::Break(count)),
        None => Ok(EvalOutcome::Code(EXIT_SYNTAX_ERROR)),
    }
}

pub fn builtin_continue(
    args: &[BString],
    _env: &mut ShellState,
    ctx: &mut ExecutionContext,
) -> Result<EvalOutcome, String> {
    match parse_loop_count("continue", args, ctx) {
        Some(count) => Ok(EvalOutcome::Continue(count)),
        None => Ok(EvalOutcome::Code(EXIT_SYNTAX_ERROR)),
    }
}

pub fn builtin_alias(
    args: &[BString],
    state: &mut ShellState,
    _stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    if args.is_empty() {
        for (alias_name, alias_value) in state.aliases.sorted_entries() {
            let _ = writeln!(stdout, "alias {}='{}'", alias_name, alias_value);
        }
        EXIT_SUCCESS
    } else {
        for arg in args {
            if let Some((name, value)) = split_key_value(arg.as_bytes()) {
                state.aliases.insert(name.to_owned(), value.to_owned());
            } else {
                if let Some(value) = state.aliases.get(arg) {
                    let _ = writeln!(stdout, "{}='{}'", arg, value);
                } else {
                    let _ = writeln!(stderr, "alias: {}: not found", arg);
                    return EXIT_FAILURE;
                }
            }
        }
        EXIT_SUCCESS
    }
}

pub fn builtin_unalias(
    args: &[BString],
    state: &mut ShellState,
    _stdin: &mut dyn Read,
    _stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    if args.is_empty() {
        let _ = writeln!(stderr, "unalias: usage: unalias [-a] name...");
        return EXIT_FAILURE;
    }
    if args[0] == "-a" {
        state.aliases.clear();
        EXIT_SUCCESS
    } else {
        for arg in args {
            if state.aliases.remove(arg).is_none() {
                let _ = writeln!(stderr, "unalias: {}: not found", arg);
                return EXIT_FAILURE;
            }
        }
        EXIT_SUCCESS
    }
}

pub fn builtin_umask(
    args: &[BString],
    state: &mut ShellState,
    _stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    if args.is_empty() {
        let _ = writeln!(stdout, "{:04o}", state.umask());
        return EXIT_SUCCESS;
    }

    if args[0] == "-S" {
        let perm = 0o777 & !state.umask();
        let format_who = |shift: u32| {
            let mode_val = (perm >> shift) & 7;
            let mut perm_str = String::new();
            if mode_val & 4 != 0 {
                perm_str.push('r');
            }
            if mode_val & 2 != 0 {
                perm_str.push('w');
            }
            if mode_val & 1 != 0 {
                perm_str.push('x');
            }
            perm_str
        };
        let _ = writeln!(stdout, "u={},g={},o={}", format_who(6), format_who(3), format_who(0));
        return EXIT_SUCCESS;
    }

    let arg = &args[0];
    let new_umask = match parse_mode_mask(arg.as_bytes(), state.umask()) {
        Some(val) => val,
        None => {
            let _ = writeln!(stderr, "umask: {}: invalid mode", arg);
            return EXIT_FAILURE;
        }
    };
    state.set_umask(new_umask);
    EXIT_SUCCESS
}

pub fn builtin_readonly(
    args: &[BString],
    state: &mut ShellState,
    _stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    if args.is_empty() {
        for name in state.readonly().sorted() {
            if let Some(val) = state.get_var(name) {
                let _ = writeln!(stdout, "readonly {}='{}'", name, val);
            } else {
                let _ = writeln!(stdout, "readonly {}", name);
            }
        }
        EXIT_SUCCESS
    } else {
        for arg in args {
            if let Some((name, value)) = split_key_value(arg.as_bytes()) {
                if state.is_readonly(name) {
                    let _ = writeln!(stderr, "readonly: {}: readonly variable", name);
                    return EXIT_FAILURE;
                }
                state.set_var(name, value);
                state.make_readonly(name);
            } else {
                state.make_readonly(arg);
            }
        }
        EXIT_SUCCESS
    }
}

pub fn builtin_hash(
    args: &[BString],
    state: &mut ShellState,
    _stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    if args.is_empty() {
        let cache = state.command_cache();
        if cache.is_empty() {
            let _ = writeln!(stdout, "hash: hash table empty");
            return EXIT_SUCCESS;
        }
        let _ = writeln!(stdout, "hits\tcommand");
        for (_cmd_name, entry) in cache.sorted_entries() {
            let _ = writeln!(stdout, "{:>4}\t{}", entry.hits, entry.path);
        }
        return EXIT_SUCCESS;
    }

    if args[0] == "-r" {
        state.clear_command_cache();
        return EXIT_SUCCESS;
    }

    let mut status = EXIT_SUCCESS;
    for cmd in args {
        if cmd.starts_with(b"-") {
            let _ = writeln!(stderr, "hash: {}: invalid option", cmd);
            return EXIT_FAILURE;
        }
        if cmd.find(b"/").is_some() {
            continue;
        }
        let mut found = false;
        if let Some(resolved) = state.path().resolve(cmd.as_ref()) {
            state.insert_command_cache(cmd.clone(), resolved, 0);
            found = true;
        }
        if !found {
            let _ = writeln!(stderr, "zxsh: hash: {}: not found", cmd);
            status = EXIT_FAILURE;
        }
    }
    status
}

fn display_limit(
    resource: i32,
    unit_scale: u64,
    label: &str,
    state: &ShellState,
    out: &mut dyn Write,
) -> Result<(), String> {
    let limit = state
        .get_rlimit(resource)
        .ok_or_else(|| format!("ulimit: failed to get limit for {}", label))?;
    if limit.soft == RLIM_INFINITY {
        let _ = writeln!(out, "unlimited");
    } else {
        let val = limit.soft / unit_scale;
        let _ = writeln!(out, "{}", val);
    }
    Ok(())
}

fn display_all_limits(state: &ShellState, out: &mut dyn Write) -> Result<(), String> {
    let mut print_one =
        |resource: i32, unit_scale: u64, label: &str, flag: &str| -> Result<(), String> {
            let limit = state
                .get_rlimit(resource)
                .ok_or_else(|| format!("ulimit: failed to get limit for {}", label))?;
            if limit.soft == RLIM_INFINITY {
                let _ = writeln!(out, "{:<30} (-{}) unlimited", label, flag);
            } else {
                let val = limit.soft / unit_scale;
                let _ = writeln!(out, "{:<30} (-{}) {}", label, flag, val);
            }
            Ok(())
        };

    print_one(RLIMIT_CORE, 512, "core file size (blocks)", "c")?;
    print_one(RLIMIT_FSIZE, 512, "file size (blocks)", "f")?;
    print_one(RLIMIT_NOFILE, 1, "open files", "n")?;
    Ok(())
}

fn set_limit(
    resource: i32,
    unit_scale: u64,
    val_str: &BStr,
    state: &mut ShellState,
) -> Result<(), String> {
    let val_num = if val_str == "unlimited" {
        RLIM_INFINITY
    } else {
        let val = parse_int::<u64>(val_str.as_bytes())
            .ok_or_else(|| "ulimit: invalid limit value".to_string())?;
        val * unit_scale
    };
    let new_val = crate::eval::Rlimit { soft: val_num, hard: val_num };
    state.set_rlimit(resource, new_val);
    Ok(())
}

pub fn builtin_ulimit(
    args: &[BString],
    state: &mut ShellState,
    _stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    if !args.is_empty() && (args[0] == "-h" || args[0] == "--help") {
        let help_text = "\
Usage: ulimit [-SHacdfilmnpstuv] [limit]
Get and set resource limits.

Supported options:
  -f    Maximum size of files written by the shell and its children (512-byte blocks)
  -n    Maximum number of open file descriptors
  -c    Maximum size of core files created (512-byte blocks)
  -a    All current limits are written to standard output
";
        let _ = write!(stdout, "{}", help_text);
        return EXIT_SUCCESS;
    }

    let mut val_arg = None;
    let flag = if !args.is_empty() && args[0].starts_with(b"-") {
        if args.len() > 1 {
            val_arg = Some(&args[1]);
        }
        args[0].as_bytes()
    } else {
        if !args.is_empty() {
            val_arg = Some(&args[0]);
        }
        b"-f".as_slice()
    };

    let resource = match flag {
        b"-f" => RLIMIT_FSIZE,
        b"-n" => RLIMIT_NOFILE,
        b"-c" => RLIMIT_CORE,
        b"-a" => {
            if val_arg.is_some() {
                let _ = writeln!(stderr, "ulimit: cannot set limit when displaying all");
                return EXIT_FAILURE;
            }
            if let Err(err) = display_all_limits(state, stdout) {
                let _ = writeln!(stderr, "{}", err);
                return EXIT_FAILURE;
            }
            return EXIT_SUCCESS;
        }
        _ => {
            let _ = writeln!(stderr, "ulimit: {}: invalid option", BStr::new(flag));
            return EXIT_FAILURE;
        }
    };

    let scale = match flag {
        b"-f" | b"-c" => 512,
        b"-n" => 1,
        _ => 1,
    };

    if let Some(val_str) = val_arg {
        if let Err(err) = set_limit(resource, scale, val_str.as_ref(), state) {
            let _ = writeln!(stderr, "{}", err);
            return EXIT_FAILURE;
        }
    } else {
        let label = match flag {
            b"-f" => "file size",
            b"-n" => "open files",
            b"-c" => "core size",
            _ => "limit",
        };
        if let Err(err) = display_limit(resource, scale, label, state, stdout) {
            let _ = writeln!(stderr, "{}", err);
            return EXIT_FAILURE;
        }
    }

    EXIT_SUCCESS
}
