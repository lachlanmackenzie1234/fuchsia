// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! Hermetic mock driver test harness and unit test suite for `ffx_tool_uart`.
//!
//! This module verifies host CLI behaviors without requiring physical hardware or running
//! system background daemons. It simulates driver lifecycles, control sockets, and metadata
//! persistence within isolated temporary test environments.

use argh::FromArgs;

use super::*;
use crate::args::{ConnectCommand, DisconnectCommand};
use ffx_writer::TestBuffers;
use fho::FfxMain;
use fuchsia_async as _;
use nix::sys::signal::kill;
use nix::unistd::Pid;
use sha2::Digest;
use std::fs;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

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

fn make_connect_cmd(baud: u32, reconnect: bool) -> UartSubCommand {
    UartSubCommand::Connect(ConnectCommand {
        no_retry: false,
        baud: NonZeroU32::new(baud),
        socket: None,
        reconnect,
        protocol: None,
    })
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

#[fuchsia::test]
async fn test_metadata_serialization_deserialization() {
    let (env, _temp_dir) = create_test_env("test_metadata_serialization_deserialization");
    let target = "/test-serial-deserial";
    let _guard = DaemonCleanupGuard::new(&env, target);
    let pid = 12345;
    let socket_path = get_socket_path(&env.context, target).unwrap();

    write_metadata(&env.context, &socket_path, target, pid, NonZeroU32::new(115200)).unwrap();

    let meta = read_metadata(&env.context, target).unwrap().unwrap();
    assert_eq!(meta.pid, pid);
    assert_eq!(meta.target, target);
    assert_eq!(meta.baud, NonZeroU32::new(115200));

    delete_metadata(&env.context, target).unwrap();
    assert!(read_metadata(&env.context, target).unwrap().is_none());
}

#[fuchsia::test]
async fn test_liveness_check() {
    let current_pid = std::process::id();
    assert!(is_running(current_pid));

    assert!(!is_running(0));
    assert!(!is_running(u32::MAX));
}

#[fuchsia::test]
async fn test_corrupt_metadata_recovery() {
    let (env, _temp_dir) = create_test_env("test_corrupt_metadata_recovery");
    let target = "/test-corrupt-target";
    let _guard = DaemonCleanupGuard::new(&env, target);
    let path = get_metadata_path(&env.context, target).unwrap();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    // Write corrupt JSON content
    fs::write(&path, b"invalid json content").unwrap();

    let meta = read_metadata(&env.context, target).unwrap().unwrap();
    assert_eq!(meta.pid, 0);
    assert_eq!(meta.target, target);
    assert!(!is_running(meta.pid));
}

#[fuchsia::test]
async fn test_connect_handles_corrupt_metadata() {
    let (env, _temp_dir) = create_test_env("test_connect_handles_corrupt_metadata");
    let target = "/test-corrupt-connect-target";
    let _guard = DaemonCleanupGuard::new(&env, target);
    let path = get_metadata_path(&env.context, target).unwrap();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    // Write corrupt JSON content
    fs::write(&path, b"invalid json content").unwrap();

    let sub_cmd = make_connect_cmd(115200, false);

    // This should succeed because the corrupt metadata is treated as not running (pid = 0)
    let (stdout, _) = run_tool(&env, Some(target), sub_cmd).await.unwrap();
    assert_eq!(format!("Connect called for {}\n", target), stdout);

    // Verify metadata was successfully written with valid JSON and our process ID
    let metadata = read_metadata(&env.context, target).unwrap().unwrap();
    assert_eq!(metadata.target, canonicalize_target(target));
    assert!(metadata.pid > 0);
}

#[fuchsia::test]
async fn test_connect_state_transitions() {
    let (env, _temp_dir) = create_test_env("test_connect_state_transitions");
    let target = "/test-transition-target";
    let _guard = DaemonCleanupGuard::new(&env, target);

    // 1. Initial connect -> success
    run_tool(&env, Some(target), make_connect_cmd(115200, false)).await.unwrap();

    // 2. Connect again without reconnect flag -> fail (Already connected)
    let err = run_tool(&env, Some(target), make_connect_cmd(115200, false)).await.unwrap_err();
    assert!(err.to_string().contains("Already connected to target"));

    // 3. Connect again with reconnect flag -> success
    run_tool(&env, Some(target), make_connect_cmd(115200, true)).await.unwrap();

    // 4. Disconnect -> success
    let (stdout, _) =
        run_tool(&env, Some(target), UartSubCommand::Disconnect(DisconnectCommand {}))
            .await
            .unwrap();
    assert_eq!("Disconnect called for /test-transition-target\n", stdout);

    // 5. Disconnect again -> fail (Not connected)
    let err_disc2 = run_tool(&env, Some(target), UartSubCommand::Disconnect(DisconnectCommand {}))
        .await
        .unwrap_err();
    assert!(err_disc2.to_string().contains("Not connected to target"));

    // 6. Connect again -> success
    run_tool(&env, Some(target), make_connect_cmd(115200, false)).await.unwrap();
}

#[fuchsia::test]
async fn test_reconnect_argument_merging() {
    let (env1, _temp_dir) =
        create_test_env_with_log_levels("reconnect_argument_merging", Some("debug"), Some("info"));

    let target = "/test-reconnect-merge-target";
    let _guard = DaemonCleanupGuard::new(&env1, target);

    // Initial connect with baud=115200
    run_tool(&env1, Some(target), make_connect_cmd(115200, false)).await.unwrap();

    let meta = read_metadata(&env1.context, target).unwrap().unwrap();
    assert_eq!(meta.baud, NonZeroU32::new(115200));

    // Reconnect without specifying baud inherits previous baud (115200)
    let recon_cmd = UartSubCommand::Connect(ConnectCommand {
        no_retry: false,
        baud: None,
        socket: None,
        reconnect: true,
        protocol: None,
    });
    run_tool(&env1, Some(target), recon_cmd).await.unwrap();

    let meta2 = read_metadata(&env1.context, target).unwrap().unwrap();
    assert_eq!(meta2.baud, NonZeroU32::new(115200));
}

#[fuchsia::test]
async fn test_disconnect_cleans_up_stale_files() {
    let (env, _temp_dir) = create_test_env("test_disconnect_stale");
    let target = "/test-disconnect-stale-target";
    let _guard = DaemonCleanupGuard::new(&env, target);

    let conn_cmd = make_connect_cmd(115200, false);
    run_tool(&env, Some(target), conn_cmd).await.unwrap();

    let meta_path = get_metadata_path(&env.context, target).unwrap();
    let socket_path = get_socket_path(&env.context, target).unwrap();
    let control_path = uart_driver_api::get_control_socket_path(&socket_path);
    assert!(meta_path.exists());
    assert!(socket_path.exists());
    assert!(control_path.exists());

    run_tool(&env, Some(target), UartSubCommand::Disconnect(DisconnectCommand {})).await.unwrap();

    assert!(!meta_path.exists());
    assert!(!socket_path.exists());
    assert!(!control_path.exists());
}

#[fuchsia::test]
async fn test_disconnect_by_target_id() {
    let (env, _temp_dir) = create_test_env("test_disconnect_target_id");
    let target = "/dev/ttyUSB99";
    let _guard = DaemonCleanupGuard::new(&env, target);

    let conn_cmd = make_connect_cmd(115200, false);
    run_tool(&env, Some(target), conn_cmd).await.unwrap();

    let canonical = canonicalize_target(target);
    let target_id = get_target_id(&canonical);

    // Disconnect using the 16-hex target ID
    let (stdout, _) =
        run_tool(&env, Some(&target_id), UartSubCommand::Disconnect(DisconnectCommand {}))
            .await
            .unwrap();
    assert!(stdout.contains(&format!("Disconnect called for {}", target)));

    assert!(read_metadata(&env.context, target).unwrap().is_none());
}

#[fuchsia::test]
async fn test_disconnect_pattern_matching_tty_usb0() {
    // Verify that disconnecting by device filename (e.g. "ttyUSB0") correctly matches "/dev/ttyUSB0".
    let (env, _temp_dir) = create_test_env("test_disconnect_pattern_matching");
    let target = "/dev/ttyUSB0";
    let _guard = DaemonCleanupGuard::new(&env, target);

    let conn_cmd = make_connect_cmd(1_000_000, false);
    run_tool(&env, Some(target), conn_cmd).await.unwrap();

    // Disconnect using "ttyUSB0" without "/dev/" prefix
    let (stdout, _) =
        run_tool(&env, Some("ttyUSB0"), UartSubCommand::Disconnect(DisconnectCommand {}))
            .await
            .unwrap();
    assert_eq!("Disconnect called for /dev/ttyUSB0\n", stdout);

    assert!(read_metadata(&env.context, target).unwrap().is_none());
}

#[fuchsia::test]
async fn test_terminate_process_kills_child() {
    // Verify that terminate_process cleanly sends SIGTERM/SIGKILL to terminate a child process.
    let mut child = std::process::Command::new("sleep").arg("60").spawn().unwrap();
    let pid = child.id();
    let res = crate::driver::terminate_process(pid).await;
    assert!(res.is_ok());

    let start = std::time::Instant::now();
    let mut exited = false;
    while start.elapsed() < std::time::Duration::from_secs(2) {
        if let Ok(Some(_)) = child.try_wait() {
            exited = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    if !exited {
        let _ = child.kill();
    }
    assert!(exited, "Child process should have exited after terminate_process");
}

#[fuchsia::test]
async fn test_socket_path_length_limit() {
    let (env, _temp_dir) = create_test_env("test_socket_path_len");
    let _guard = DaemonCleanupGuard::new(&env, "placeholder");

    // Construct a socket path exceeding the OS limit (> 108 bytes on Linux, > 104 on macOS)
    let long_socket = format!("/tmp/{}", "a".repeat(120));
    let conn_cmd = UartSubCommand::Connect(ConnectCommand {
        no_retry: false,
        baud: NonZeroU32::new(115200),
        socket: Some(long_socket),
        reconnect: false,
        protocol: None,
    });

    let res = run_tool(&env, Some("/dev/test_long"), conn_cmd).await;
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(err.to_string().contains("UNIX socket path exceeds limit"));
}
