// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! Hermetic mock driver test harness and unit test suite for `ffx_tool_uart`.
//!
//! This module verifies host CLI behaviors without requiring physical hardware or running
//! system background daemons. It simulates driver lifecycles, control sockets, and metadata
//! persistence within isolated temporary test environments.

use argh::FromArgs;
use ffx_writer::TestBuffers;
use fho::FfxMain;
use fuchsia_async as _;
use nix::sys::signal::kill;
use nix::unistd::Pid;
use sha2::Digest;
use std::fs;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

use super::*;
use crate::args::{ConnectCommand, DisconnectCommand};

const MOCK_DRIVER_SCRIPT: &str = include_str!("../test_data/mock_driver.py");

fn write_mock_driver(temp_path: &Path, filename: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script_path = temp_path.join(filename);
    fs::write(&script_path, MOCK_DRIVER_SCRIPT).unwrap();
    let mut perms = fs::metadata(&script_path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script_path, perms).unwrap();
    script_path
}

fn create_test_env_with_log_levels(
    test_name: &str,
    runtime_log: Option<&str>,
    user_log: Option<&str>,
) -> (ffx_config::TestEnv, tempfile::TempDir) {
    let temp_dir = tempfile::Builder::new()
        .prefix(&format!("u_{}_{}_", std::process::id(), test_name))
        .tempdir()
        .expect("create temp dir");

    let script_path = write_mock_driver(temp_dir.path(), "mock_driver.py");

    let mut builder = ffx_config::test_env()
        .runtime_config("shared_data", temp_dir.path().to_str().unwrap())
        .runtime_config("sdk.overrides.ffx-uart-driver", script_path.to_str().unwrap());
    if let Some(rl) = runtime_log {
        builder = builder.runtime_config("log.level", rl);
    }
    if let Some(ul) = user_log {
        builder = builder.user_config("log.level", ul);
    }
    let env = builder.build().expect("test env");
    (env, temp_dir)
}

fn create_test_env(test_name: &str) -> (ffx_config::TestEnv, tempfile::TempDir) {
    create_test_env_with_log_levels(test_name, None, None)
}

async fn run_tool(
    env: &ffx_config::TestEnv,
    target: Option<&str>,
    sub_cmd: UartSubCommand,
) -> std::result::Result<(String, String), fho::Error> {
    let mut context_tool = env.context.clone();
    if let Some(target) = target {
        context_tool.override_target_specifier(&Some(format!("uart:{}", target)));
    }
    let buffers = TestBuffers::default();
    let writer = ffx_writer::SimpleWriter::new_test(&buffers);
    match sub_cmd {
        UartSubCommand::Connect(cmd) => {
            let tool = ConnectTool { cmd, context: context_tool };
            tool.main(writer).await?;
        }
        UartSubCommand::Disconnect(cmd) => {
            let tool = DisconnectTool { cmd, context: context_tool };
            tool.main(writer).await?;
        }
        UartSubCommand::List(cmd) => {
            let tool = ListTool { cmd, context: context_tool };
            let machine_writer = ffx_writer::MachineWriter::new_test(None, &buffers);
            tool.main(machine_writer).await?;
        }
        UartSubCommand::Probe(cmd) => {
            let tool = ProbeTool { cmd, context: context_tool };
            tool.main(writer).await?;
        }
        UartSubCommand::Status(cmd) => {
            let tool = StatusTool { cmd, context: context_tool };
            tool.main(writer).await?;
        }
    }
    Ok((buffers.stdout.into_string(), buffers.stderr.into_string()))
}

struct DaemonCleanupGuard<'a> {
    env: &'a ffx_config::TestEnv,
    canonical_target: String,
}

impl<'a> DaemonCleanupGuard<'a> {
    fn new(env: &'a ffx_config::TestEnv, target: &str) -> Self {
        let canonical_target = canonicalize_target(target);
        Self { env, canonical_target }
    }
}

impl<'a> Drop for DaemonCleanupGuard<'a> {
    fn drop(&mut self) {
        if let Ok(Some(meta)) = read_metadata(&self.env.context, &self.canonical_target) {
            if is_running(meta.pid) {
                let nix_pid = Pid::from_raw(meta.pid as i32);
                let _ = kill(nix_pid, nix::sys::signal::Signal::SIGKILL);
            }
        }
    }
}

#[fuchsia::test]
async fn test_connect() {
    let (env, _temp_dir) = create_test_env("test_connect");
    let _guard = DaemonCleanupGuard::new(&env, "/target-connect");

    let sub_cmd = UartSubCommand::Connect(ConnectCommand {
        no_retry: false,
        baud: NonZeroU32::new(115200),
        socket: None,
        reconnect: false,
        protocol: None,
    });

    let (stdout, _) = run_tool(&env, Some("/target-connect"), sub_cmd).await.unwrap();
    assert_eq!("Connect called for /target-connect\n", stdout);

    // Verify metadata was written
    let metadata = read_metadata(&env.context, "/target-connect").unwrap().unwrap();
    assert_eq!(metadata.target, canonicalize_target("/target-connect"));
    assert!(metadata.pid > 0);
}

#[fuchsia::test]
async fn test_disconnect() {
    let (env, _temp_dir) = create_test_env("test_disconnect");
    let _guard = DaemonCleanupGuard::new(&env, "/target-disconnect");

    // First connect to create metadata
    let conn_cmd = UartSubCommand::Connect(ConnectCommand {
        no_retry: false,
        baud: NonZeroU32::new(115200),
        socket: None,
        reconnect: false,
        protocol: None,
    });
    run_tool(&env, Some("/target-disconnect"), conn_cmd).await.unwrap();

    let (stdout, _) = run_tool(
        &env,
        Some("/target-disconnect"),
        UartSubCommand::Disconnect(DisconnectCommand {}),
    )
    .await
    .unwrap();
    assert_eq!("Disconnect called for /target-disconnect\n", stdout);

    // Verify metadata was deleted
    assert!(read_metadata(&env.context, "/target-disconnect").unwrap().is_none());
}

#[fuchsia::test]
fn test_parse_connect_success() {
    let args = ["connect", "--baud", "9600"];
    let cmd = UartCommand::from_args(&["uart"], &args).unwrap();
    assert_eq!(
        cmd.sub_cmd,
        UartSubCommand::Connect(ConnectCommand {
            no_retry: false,
            baud: NonZeroU32::new(9600),
            socket: None,
            reconnect: false,
            protocol: None,
        })
    );
}

#[fuchsia::test]
fn test_parse_connect_default_baud() {
    let args = ["connect"];
    let cmd = UartCommand::from_args(&["uart"], &args).unwrap();
    assert_eq!(
        cmd.sub_cmd,
        UartSubCommand::Connect(ConnectCommand {
            no_retry: false,
            baud: None,
            socket: None,
            reconnect: false,
            protocol: None,
        })
    );
}

#[fuchsia::test]
fn test_parse_connect_invalid_baud() {
    let args = ["connect", "--baud", "abc"];
    let res = UartCommand::from_args(&["uart"], &args);
    assert!(res.is_err());
    let err_msg = res.unwrap_err().output;
    assert!(err_msg.contains("invalid digit found in string"), "Error: {}", err_msg);
}

#[fuchsia::test]
fn test_parse_connect_negative_baud() {
    let args = ["connect", "--baud", "-115200"];
    let res = UartCommand::from_args(&["uart"], &args);
    assert!(res.is_err());
    let err_msg = res.unwrap_err().output;
    assert!(err_msg.contains("invalid digit found in string"), "Error: {}", err_msg);
}

#[fuchsia::test]
fn test_parse_connect_overflow_baud() {
    let args = ["connect", "--baud", "999999999999999999999999999999"];
    let res = UartCommand::from_args(&["uart"], &args);
    assert!(res.is_err());
    let err_msg = res.unwrap_err().output;
    assert!(err_msg.contains("number too large to fit in target type"), "Error: {}", err_msg);
}

#[fuchsia::test]
fn test_parse_connect_zero_baud() {
    let args = ["connect", "--baud", "0"];
    let res = UartCommand::from_args(&["uart"], &args);
    assert!(res.is_err());
    let err_msg = res.unwrap_err().output;
    assert!(err_msg.contains("number would be zero for non-zero type"), "Error: {}", err_msg);
}

#[fuchsia::test]
fn test_parse_disconnect_extra_arg() {
    let args = ["disconnect", "/dev/ttyUSB0"];
    let res = UartCommand::from_args(&["uart"], &args);
    assert!(res.is_err());
    let err_msg = res.unwrap_err().output;
    assert!(err_msg.contains("Unrecognized argument"), "Error: {}", err_msg);
}

#[fuchsia::test]
fn test_parse_list_extra_arg() {
    let args = ["list", "/dev/ttyUSB0"];
    let res = UartCommand::from_args(&["uart"], &args);
    assert!(res.is_err());
    let err_msg = res.unwrap_err().output;
    assert!(err_msg.contains("Unrecognized argument"), "Error: {}", err_msg);
}

#[fuchsia::test]
fn test_target_id_hashing() {
    let target = "my-test-target-hashing";
    let canonical = canonicalize_target(target);
    let mut hasher = sha2::Sha256::new();
    hasher.update(canonical.as_bytes());
    let result = hasher.finalize();
    let expected_hash = hex::encode(&result[..8]);
    assert_eq!(get_target_id(target), expected_hash);
}
