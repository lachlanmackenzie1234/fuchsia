// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use super::*;
use bstr::{BStr, BString, ByteSlice};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::FromRawFd;

fn create_pipe() -> (File, File) {
    let mut fds = [0; 2];
    unsafe {
        assert_eq!(libc::pipe(fds.as_mut_ptr()), 0);
        (File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1]))
    }
}

fn test_editor() -> Editor {
    Editor::with_config(
        Config::default()
            .with_terminal_mode(TerminalMode::Tty)
            .with_column_width(ColumnWidth::AnsiCursor)
            .with_term_name(Some("xterm-256color")),
    )
}

#[test]
fn test_readline_error_traits() {
    let err_int = ReadlineError::Interrupted;
    assert_eq!(format!("{}", err_int), "Interrupted");
    assert!(std::error::Error::source(&err_int).is_none());

    let err_eof = ReadlineError::Eof;
    assert_eq!(format!("{}", err_eof), "End of file");
    assert!(std::error::Error::source(&err_eof).is_none());

    let io_err = std::io::Error::new(std::io::ErrorKind::Other, "test error");
    let err_io = ReadlineError::from(io_err);
    assert!(format!("{}", err_io).contains("I/O error"));
    assert!(std::error::Error::source(&err_io).is_some());
}

#[test]
fn test_color_enum() {
    assert_eq!(Color::Red.to_ansi_code(), 31);
    assert_eq!(Color::Green.to_ansi_code(), 32);
    assert_eq!(Color::Yellow.to_ansi_code(), 33);
    assert_eq!(Color::Blue.to_ansi_code(), 34);
    assert_eq!(Color::White.to_ansi_code(), 37);
    assert_eq!(Color::Custom(123).to_ansi_code(), 123);

    assert_eq!(Color::from(31), Color::Red);
    assert_eq!(Color::from(37), Color::White);
    assert_eq!(Color::from(42), Color::Custom(42));
}

#[test]
fn test_hint_struct() {
    let hint = Hint::new("test").with_color(Color::Red).with_bold(true);
    assert_eq!(hint.text, BString::from("test"));
    assert_eq!(hint.color, Some(Color::Red));
    assert_eq!(hint.bold, true);

    let hint_int = Hint::new("test2").with_color(32);
    assert_eq!(hint_int.color, Some(Color::Green));
}

#[test]
fn test_config_struct() {
    let cfg = Config::default();
    assert_eq!(cfg.max_history_len, DEFAULT_MAX_HISTORY_LEN);
    assert_eq!(cfg.multiline_mode, false);
    assert_eq!(cfg.max_line_len, DEFAULT_MAX_LINE_LEN);
    assert_eq!(cfg.terminal_mode, TerminalMode::Auto);
    assert_eq!(cfg.column_width, ColumnWidth::Auto);
}

#[test]
fn test_config_builder() {
    let cfg = Config::default()
        .with_terminal_mode(TerminalMode::Tty)
        .with_term_name(Some("xterm"))
        .with_column_width(ColumnWidth::Fixed(120));
    assert_eq!(cfg.terminal_mode, TerminalMode::Tty);
    assert_eq!(cfg.term_name.as_deref(), Some("xterm"));
    assert_eq!(cfg.column_width, ColumnWidth::Fixed(120));
}

#[test]
fn test_history_operations() {
    let mut history = History::new(3);
    assert_eq!(history.entries().len(), 0);

    assert!(history.add("first"));
    assert!(history.add("second"));
    assert!(!history.add("second")); // Duplicate ignored
    assert!(history.add("third"));
    assert_eq!(history.entries().len(), 3);

    assert!(history.add("fourth")); // Evicts "first"
    assert_eq!(
        history.entries(),
        &[BString::from("second"), BString::from("third"), BString::from("fourth")]
    );

    let mut zero_history = History::new(0);
    assert!(!zero_history.add("test"));
}

#[test]
fn test_editor_creation_and_handlers() {
    let mut editor = Editor::new();
    editor.set_completion_handler(|line: &BStr| {
        if line == b"f" { vec![BString::from("foo"), BString::from("bar")] } else { vec![] }
    });

    editor.set_hint_handler(|line: &BStr| {
        if line == b"foo" { Some(Hint::new("bar").with_color(32)) } else { None }
    });

    assert!(editor.completion_handler.is_some());
    assert!(editor.hint_handler.is_some());

    editor.clear_completion_handler();
    editor.clear_hint_handler();

    assert!(editor.completion_handler.is_none());
    assert!(editor.hint_handler.is_none());
}

#[test]
fn test_editor_history_public_api() {
    let mut editor = Editor::new();
    assert!(editor.history().is_empty());
    assert!(editor.add_history("cmd 1"));
    assert!(editor.add_history("cmd 2"));
    assert_eq!(editor.history().len(), 2);
    assert_eq!(editor.history().entries(), &[BString::from("cmd 1"), BString::from("cmd 2")]);

    editor.history_mut().clear();
    assert!(editor.history().is_empty());
}

#[test]
fn test_readline_long_prompt() {
    let (mut r_in, mut w_in) = create_pipe();
    let (mut r_out, mut w_out) = create_pipe();

    let drain_handle = std::thread::spawn(move || {
        let mut buf = [0u8; 1024];
        while let Ok(n) = r_out.read(&mut buf) {
            if n == 0 {
                break;
            }
        }
    });

    let handle = std::thread::spawn(move || {
        let mut editor = Editor::with_config(
            Config::default()
                .with_terminal_mode(TerminalMode::Tty)
                .with_column_width(ColumnWidth::Fixed(80))
                .with_term_name(Some("xterm-256color")),
        );
        let long_prompt = "P".repeat(200);
        let mode = editor.config.resolve_operating_mode(|| true);
        editor.readline_from(&mut r_in, &mut w_out, mode, long_prompt.as_bytes().as_bstr())
    });

    w_in.write_all(b"hello\n").unwrap();

    let res = handle.join().unwrap();
    let _ = drain_handle.join();
    assert_eq!(res.ok(), Some(BString::from("hello")));
}

#[test]
fn test_line_editing_keybindings() {
    let (mut r_in, mut w_in) = create_pipe();
    let (mut r_out, mut w_out) = create_pipe();

    let handle = std::thread::spawn(move || {
        let mut editor = test_editor();
        editor.readline_from(
            &mut r_in,
            &mut w_out,
            OperatingMode::Interactive,
            b"prompt> ".as_bstr(),
        )
    });

    let mut buf = [0u8; 32];
    let _ = r_out.read(&mut buf[..4]);
    let _ = w_in.write_all(b"\x1b[10;80R");
    let _ = r_out.read(&mut buf[..6]);
    let _ = r_out.read(&mut buf[..4]);
    let _ = w_in.write_all(b"\x1b[10;80R");

    // Type "wordl", Left (2), Transpose (20) -> "world", End (5), Enter (10)
    let _ = w_in.write_all(&[
        b'w', b'o', b'r', b'd', b'l', 2, 20, 5, 10, // Enter
    ]);

    let res = handle.join().unwrap();
    assert_eq!(res.ok(), Some(BString::from("world")));
}

#[test]
fn test_escape_sequences_and_history_navigation() {
    let (mut r_in, mut w_in) = create_pipe();
    let (mut r_out, mut w_out) = create_pipe();

    let handle = std::thread::spawn(move || {
        let mut editor = test_editor();
        editor.history.add("first_cmd");
        editor.history.add("second_cmd");
        editor.readline_from(
            &mut r_in,
            &mut w_out,
            OperatingMode::Interactive,
            b"prompt> ".as_bstr(),
        )
    });

    let mut buf = [0u8; 32];
    let _ = r_out.read(&mut buf[..4]);
    let _ = w_in.write_all(b"\x1b[10;80R");
    let _ = r_out.read(&mut buf[..6]);
    let _ = r_out.read(&mut buf[..4]);
    let _ = w_in.write_all(b"\x1b[10;80R");

    // Up Arrow (ESC [ A), Down Arrow (ESC [ B), Up Arrow (ESC [ A), Up Arrow (ESC [ A), Enter
    let _ = w_in.write_all(b"\x1b[A\x1b[B\x1b[A\x1b[A\n");

    let res = handle.join().unwrap();
    assert_eq!(res.ok(), Some(BString::from("first_cmd")));
}

#[test]
fn test_history_draft_line_preservation() {
    let (mut r_in, mut w_in) = create_pipe();
    let (mut r_out, mut w_out) = create_pipe();

    let handle = std::thread::spawn(move || {
        let mut editor = test_editor();
        editor.history.add("prev_cmd1");
        editor.history.add("prev_cmd2");
        editor.readline_from(
            &mut r_in,
            &mut w_out,
            OperatingMode::Interactive,
            b"prompt> ".as_bstr(),
        )
    });

    let mut buf = [0u8; 32];
    let _ = r_out.read(&mut buf[..4]);
    let _ = w_in.write_all(b"\x1b[10;80R");
    let _ = r_out.read(&mut buf[..6]);
    let _ = r_out.read(&mut buf[..4]);
    let _ = w_in.write_all(b"\x1b[10;80R");

    // Type partial draft "draft_command", navigate up through history, navigate back down to draft,
    // append " _appended", and submit.
    let _ = w_in.write_all(b"draft_command\x1b[A\x1b[A\x1b[B\x1b[B _appended\n");

    let res = handle.join().unwrap();
    assert_eq!(res.ok(), Some(BString::from("draft_command _appended")));
}

#[test]
fn test_ctrl_c_and_ctrl_d() {
    // Test Ctrl-C
    {
        let (mut r_in, mut w_in) = create_pipe();
        let (mut r_out, mut w_out) = create_pipe();

        let handle = std::thread::spawn(move || {
            let mut editor = test_editor();
            editor.readline_from(
                &mut r_in,
                &mut w_out,
                OperatingMode::Interactive,
                b"prompt> ".as_bstr(),
            )
        });

        let mut buf = [0u8; 32];
        let _ = r_out.read(&mut buf[..4]);
        let _ = w_in.write_all(b"\x1b[10;80R");
        let _ = r_out.read(&mut buf[..6]);
        let _ = r_out.read(&mut buf[..4]);
        let _ = w_in.write_all(b"\x1b[10;80R");

        let _ = w_in.write_all(&[3]); // Ctrl-C

        let res = handle.join().unwrap();
        assert!(matches!(res, Err(ReadlineError::Interrupted)));
    }

    // Test Ctrl-D on empty line
    {
        let (mut r_in, mut w_in) = create_pipe();
        let (mut r_out, mut w_out) = create_pipe();

        let handle = std::thread::spawn(move || {
            let mut editor = test_editor();
            editor.readline_from(
                &mut r_in,
                &mut w_out,
                OperatingMode::Interactive,
                b"prompt> ".as_bstr(),
            )
        });

        let mut buf = [0u8; 32];
        let _ = r_out.read(&mut buf[..4]);
        let _ = w_in.write_all(b"\x1b[10;80R");
        let _ = r_out.read(&mut buf[..6]);
        let _ = r_out.read(&mut buf[..4]);
        let _ = w_in.write_all(b"\x1b[10;80R");

        let _ = w_in.write_all(&[4]); // Ctrl-D

        let res = handle.join().unwrap();
        assert!(matches!(res, Err(ReadlineError::Eof)));
    }
}

#[test]
fn test_multiline_mode() {
    let (mut r_in, mut w_in) = create_pipe();
    let (mut r_out, mut w_out) = create_pipe();

    let handle = std::thread::spawn(move || {
        let mut editor = Editor::with_config(
            Config { multiline_mode: true, ..Config::default() }
                .with_terminal_mode(TerminalMode::Tty)
                .with_term_name(Some("xterm-256color"))
                .with_column_width(ColumnWidth::AnsiCursor),
        );
        editor.readline_from(
            &mut r_in,
            &mut w_out,
            OperatingMode::Interactive,
            b"prompt> ".as_bstr(),
        )
    });

    let mut buf = [0u8; 32];
    let _ = r_out.read(&mut buf[..4]);
    let _ = w_in.write_all(b"\x1b[10;80R");
    let _ = r_out.read(&mut buf[..6]);
    let _ = r_out.read(&mut buf[..4]);
    let _ = w_in.write_all(b"\x1b[10;80R");

    let _ = w_in.write_all(b"multiline_input\n");

    let res = handle.join().unwrap();
    assert_eq!(res.ok(), Some(BString::from("multiline_input")));
}

#[test]
fn test_completion_and_hints() {
    let (mut r_in, mut w_in) = create_pipe();
    let (mut r_out, mut w_out) = create_pipe();

    let handle = std::thread::spawn(move || {
        let mut editor = test_editor();
        editor.set_completion_handler(|line: &BStr| {
            if line.starts_with(b"h") {
                vec![BString::from("hello"), BString::from("help")]
            } else {
                vec![]
            }
        });
        editor.set_hint_handler(|line: &BStr| {
            if line == b"hello" {
                Some(Hint::new(" world").with_color(33).with_bold(true))
            } else {
                None
            }
        });
        editor.readline_from(
            &mut r_in,
            &mut w_out,
            OperatingMode::Interactive,
            b"prompt> ".as_bstr(),
        )
    });

    let mut buf = [0u8; 32];
    let _ = r_out.read(&mut buf[..4]);
    let _ = w_in.write_all(b"\x1b[10;80R");
    let _ = r_out.read(&mut buf[..6]);
    let _ = r_out.read(&mut buf[..4]);
    let _ = w_in.write_all(b"\x1b[10;80R");

    // Type 'h', TAB (9), ENTER (10)
    let _ = w_in.write_all(&[b'h', 9, 10]);

    let res = handle.join().unwrap();
    assert_eq!(res.ok(), Some(BString::from("hello")));
}

#[test]
fn test_binary_non_utf8() {
    let (mut r_in, mut w_in) = create_pipe();
    let (mut r_out, mut w_out) = create_pipe();

    let handle = std::thread::spawn(move || {
        let mut editor = test_editor();
        editor.readline_from(
            &mut r_in,
            &mut w_out,
            OperatingMode::Interactive,
            b"prompt> ".as_bstr(),
        )
    });

    let mut buf = [0u8; 32];
    let _ = r_out.read(&mut buf[..4]);
    let _ = w_in.write_all(b"\x1b[10;80R");
    let _ = r_out.read(&mut buf[..6]);
    let _ = r_out.read(&mut buf[..4]);
    let _ = w_in.write_all(b"\x1b[10;80R");

    let _ = w_in.write_all(&[0xFF, 0xFE, 0xFD, 10]);

    let res = handle.join().unwrap();
    assert_eq!(res.ok(), Some(BString::from(&[0xFF, 0xFE, 0xFD][..])));
}

#[test]
fn test_completion_empty_and_wrap_beep() {
    let (mut r_in, mut w_in) = create_pipe();
    let (mut r_out, mut w_out) = create_pipe();

    let handle = std::thread::spawn(move || {
        let mut editor = test_editor();
        // Completion returning empty list
        editor.set_completion_handler(|_line: &BStr| vec![]);
        editor.readline_from(
            &mut r_in,
            &mut w_out,
            OperatingMode::Interactive,
            b"prompt> ".as_bstr(),
        )
    });

    let mut buf = [0u8; 32];
    let _ = r_out.read(&mut buf[..4]);
    let _ = w_in.write_all(b"\x1b[10;80R");
    let _ = r_out.read(&mut buf[..6]);
    let _ = r_out.read(&mut buf[..4]);
    let _ = w_in.write_all(b"\x1b[10;80R");

    // Press TAB on empty completions, then enter
    let _ = w_in.write_all(&[9, b'a', 10]);

    let res = handle.join().unwrap();
    assert_eq!(res.ok(), Some(BString::from("a")));
}

#[test]
fn test_completion_cycling_wrap() {
    let (mut r_in, mut w_in) = create_pipe();
    let (mut r_out, mut w_out) = create_pipe();

    let handle = std::thread::spawn(move || {
        let mut editor = test_editor();
        editor.set_completion_handler(|_line: &BStr| vec![BString::from("one")]);
        editor.readline_from(
            &mut r_in,
            &mut w_out,
            OperatingMode::Interactive,
            b"prompt> ".as_bstr(),
        )
    });

    let mut buf = [0u8; 32];
    let _ = r_out.read(&mut buf[..4]);
    let _ = w_in.write_all(b"\x1b[10;80R");
    let _ = r_out.read(&mut buf[..6]);
    let _ = r_out.read(&mut buf[..4]);
    let _ = w_in.write_all(b"\x1b[10;80R");

    // Press TAB once ("one"), TAB twice (restores ""), then Enter
    let _ = w_in.write_all(&[9, 9, 10]);

    let res = handle.join().unwrap();
    assert_eq!(res.ok(), Some(BString::from("")));
}

#[test]
fn test_completion_esc_cancel() {
    let (mut r_in, mut w_in) = create_pipe();
    let (mut r_out, mut w_out) = create_pipe();

    let handle = std::thread::spawn(move || {
        let mut editor = test_editor();
        editor.set_completion_handler(|_line: &BStr| vec![BString::from("one")]);
        editor.readline_from(
            &mut r_in,
            &mut w_out,
            OperatingMode::Interactive,
            b"prompt> ".as_bstr(),
        )
    });

    let mut buf = [0u8; 32];
    let _ = r_out.read(&mut buf[..4]);
    let _ = w_in.write_all(b"\x1b[10;80R");
    let _ = r_out.read(&mut buf[..6]);
    let _ = r_out.read(&mut buf[..4]);
    let _ = w_in.write_all(b"\x1b[10;80R");

    // Press TAB (shows "one"), then ESC (27, cancels back to ""), then Enter
    let _ = w_in.write_all(&[9, 27, 10]);

    let res = handle.join().unwrap();
    assert_eq!(res.ok(), Some(BString::from("")));
}

#[test]
fn test_clear_screen_method() {
    let editor = Editor::new();
    assert!(editor.clear_screen().is_ok());
}

#[test]
fn test_terminal_support_checks() {
    assert_eq!(terminal_capability_from_name(Some("dumb")), TerminalCapability::PromptOnly);
    assert_eq!(terminal_capability_from_name(Some("cons25")), TerminalCapability::PromptOnly);
    assert_eq!(terminal_capability_from_name(Some("emacs")), TerminalCapability::PromptOnly);
    assert_eq!(terminal_capability_from_name(Some("uart")), TerminalCapability::UartEcho);
    assert_eq!(
        terminal_capability_from_name(Some("xterm-256color")),
        TerminalCapability::Supported
    );
    assert_eq!(terminal_capability_from_name(None), TerminalCapability::Supported);
}

#[test]
fn test_resolve_operating_mode() {
    let cfg = Config::default();
    assert_eq!(cfg.resolve_operating_mode(|| false), OperatingMode::NonTty);
    assert_eq!(cfg.resolve_operating_mode(|| true), OperatingMode::Interactive);

    let cfg_dumb =
        Config::default().with_terminal_mode(TerminalMode::Tty).with_term_name(Some("dumb"));
    assert_eq!(cfg_dumb.resolve_operating_mode(|| false), OperatingMode::PromptOnly);

    let cfg_uart =
        Config::default().with_terminal_mode(TerminalMode::Tty).with_term_name(Some("uart"));
    assert_eq!(cfg_uart.resolve_operating_mode(|| false), OperatingMode::UartEcho);

    let cfg_nontty = Config::default().with_terminal_mode(TerminalMode::NonTty);
    assert_eq!(cfg_nontty.resolve_operating_mode(|| true), OperatingMode::NonTty);
}

#[test]
fn test_readline_fallback_stream() {
    let (r_in, mut w_in) = create_pipe();
    let (mut r_out, mut w_out) = create_pipe();

    let handle = std::thread::spawn(move || {
        let mut editor = Editor::new();
        editor.readline_fallback_mode(
            r_in,
            &mut w_out,
            OperatingMode::PromptOnly,
            b"prompt> ".as_bstr(),
        )
    });

    w_in.write_all(b"fallback input\n").unwrap();

    let res = handle.join().unwrap();
    assert_eq!(res.ok(), Some(BString::from("fallback input")));

    let mut buf = [0u8; 32];
    let n = r_out.read(&mut buf).unwrap();
    assert_eq!(&buf[..n], b"prompt> ");
}

#[test]
fn test_custom_stream_isolation_from_stdin_tty() {
    // Validate that readline_from performs interactive line editing on custom streams
    // regardless of whether process std::io::stdin().is_terminal() is true or false.
    let (mut r_in, mut w_in) = create_pipe();
    let (mut r_out, mut w_out) = create_pipe();

    let handle = std::thread::spawn(move || {
        let mut editor = test_editor();
        editor.readline_from(
            &mut r_in,
            &mut w_out,
            OperatingMode::Interactive,
            b"custom_prompt> ".as_bstr(),
        )
    });

    // Interactive readline_stream writes cursor position query \x1b[6n to the custom output stream
    let mut buf = [0u8; 32];
    r_out.read_exact(&mut buf[..4]).unwrap();
    assert_eq!(&buf[..4], b"\x1b[6n");

    // Send cursor position response back to custom input stream
    w_in.write_all(b"\x1b[10;80R").unwrap();

    // Read column width query \x1b[999C and position query \x1b[6n
    r_out.read_exact(&mut buf[..6]).unwrap();
    assert_eq!(&buf[..6], b"\x1b[999C");
    r_out.read_exact(&mut buf[..4]).unwrap();
    assert_eq!(&buf[..4], b"\x1b[6n");
    w_in.write_all(b"\x1b[10;80R").unwrap();

    // Send typed input and newline over custom stream
    w_in.write_all(b"test_input\n").unwrap();

    let res = handle.join().unwrap();
    assert_eq!(res.ok(), Some(BString::from("test_input")));
}

#[test]
fn test_term_is_tty_override_false() {
    let (r_in, mut w_in) = create_pipe();
    let (mut r_out, mut w_out) = create_pipe();

    let handle = std::thread::spawn(move || {
        let mut editor =
            Editor::with_config(Config::default().with_terminal_mode(TerminalMode::NonTty));
        let mode = editor.config.resolve_operating_mode(|| true);
        assert_eq!(mode, OperatingMode::NonTty);
        editor.readline_from(r_in, &mut w_out, mode, b"prompt> ".as_bstr())
    });

    w_in.write_all(b"non_tty_input\n").unwrap();

    let res = handle.join().unwrap();
    assert_eq!(res.ok(), Some(BString::from("non_tty_input")));

    let mut buf = [0u8; 32];
    let n = r_out.read(&mut buf).unwrap();
    assert_eq!(n, 0);
}

#[test]
fn test_term_columns_override() {
    let (mut r_in, mut w_in) = create_pipe();
    let (_r_out, mut w_out) = create_pipe();

    let handle = std::thread::spawn(move || {
        let mut editor = Editor::with_config(
            Config::default()
                .with_terminal_mode(TerminalMode::Tty)
                .with_term_name(Some("xterm-256color"))
                .with_column_width(ColumnWidth::Fixed(100)),
        );
        let mode = editor.config.resolve_operating_mode(|| true);
        editor.readline_from(&mut r_in, &mut w_out, mode, b"prompt> ".as_bstr())
    });

    // Since column_width is overridden to Fixed(100), no cursor position query (\x1b[6n) is sent!
    w_in.write_all(b"fixed_cols_input\n").unwrap();

    let res = handle.join().unwrap();
    assert_eq!(res.ok(), Some(BString::from("fixed_cols_input")));
}

#[test]
fn test_term_name_override_dumb() {
    let (mut r_in, mut w_in) = create_pipe();
    let (mut r_out, mut w_out) = create_pipe();

    let handle = std::thread::spawn(move || {
        let mut editor = Editor::with_config(
            Config::default().with_terminal_mode(TerminalMode::Tty).with_term_name(Some("dumb")),
        );
        let mode = editor.config.resolve_operating_mode(|| true);
        assert_eq!(mode, OperatingMode::PromptOnly);
        editor.readline_from(&mut r_in, &mut w_out, mode, b"prompt> ".as_bstr())
    });

    w_in.write_all(b"dumb_term_input\n").unwrap();

    let res = handle.join().unwrap();
    assert_eq!(res.ok(), Some(BString::from("dumb_term_input")));

    let mut buf = [0u8; 32];
    let n = r_out.read(&mut buf).unwrap();
    assert_eq!(&buf[..n], b"prompt> ");
}

#[test]
fn test_term_name_override_uart() {
    let (mut r_in, mut w_in) = create_pipe();
    let (mut r_out, mut w_out) = create_pipe();

    let handle = std::thread::spawn(move || {
        let mut editor = Editor::with_config(
            Config::default().with_terminal_mode(TerminalMode::Tty).with_term_name(Some("uart")),
        );
        let mode = editor.config.resolve_operating_mode(|| true);
        assert_eq!(mode, OperatingMode::UartEcho);
        editor.readline_from(&mut r_in, &mut w_out, mode, b"prompt> ".as_bstr())
    });

    w_in.write_all(b"ab\n").unwrap();

    let res = handle.join().unwrap();
    assert_eq!(res.ok(), Some(BString::from("ab")));

    let mut buf = [0u8; 32];
    let n = r_out.read(&mut buf).unwrap();
    assert_eq!(&buf[..n], b"prompt> ab\n");
}

#[test]
fn test_control_sequence_builder() {
    let mut buf = Vec::new();
    control::ControlSequenceBuilder::new()
        .arg(6)
        .cmd(control::CMD_DEVICE_STATUS_REPORT)
        .write(&mut buf)
        .unwrap();
    assert_eq!(buf, b"\x1b[6n");

    buf.clear();
    control::ControlSequenceBuilder::new()
        .arg(1)
        .arg(31)
        .arg(49)
        .cmd(control::CMD_SELECT_GRAPHIC_RENDITION)
        .write(&mut buf)
        .unwrap();
    assert_eq!(buf, b"\x1b[1;31;49m");
}

#[test]
fn test_control_write_helpers() {
    let mut buf = Vec::new();
    control::write_query_cursor_position(&mut buf).unwrap();
    assert_eq!(buf, b"\x1b[6n");

    buf.clear();
    control::write_query_column_width(&mut buf).unwrap();
    assert_eq!(buf, b"\x1b[999C");

    buf.clear();
    control::write_clear_screen(&mut buf).unwrap();
    assert_eq!(buf, b"\x1b[H\x1b[2J");

    buf.clear();
    control::write_clear_to_eol(&mut buf).unwrap();
    assert_eq!(buf, b"\x1b[0K");

    buf.clear();
    control::write_reset_sgr(&mut buf).unwrap();
    assert_eq!(buf, b"\x1b[0m");

    buf.clear();
    control::write_sgr_formatting(&mut buf, true, Some(Color::Red)).unwrap();
    assert_eq!(buf, b"\x1b[1;31;49m");

    buf.clear();
    control::write_move_cursor_column(&mut buf, 10).unwrap();
    assert_eq!(buf, b"\r\x1b[10C");

    buf.clear();
    control::write_move_cursor_up(&mut buf, 3).unwrap();
    assert_eq!(buf, b"\x1b[3A");

    buf.clear();
    control::write_move_cursor_down(&mut buf, 2).unwrap();
    assert_eq!(buf, b"\x1b[2B");

    buf.clear();
    control::write_clear_line_and_move_up(&mut buf).unwrap();
    assert_eq!(buf, b"\r\x1b[0K\x1b[1A");

    buf.clear();
    control::write_erase_previous_char_uart(&mut buf).unwrap();
    assert_eq!(buf, b"\x08 \x08");
}
