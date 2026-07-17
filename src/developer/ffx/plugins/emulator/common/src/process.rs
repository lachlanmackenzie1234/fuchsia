// Copyright 2021 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! This module contains utility functions for process control.

use anyhow::{Result, bail};
use shared_child::SharedChild;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{thread, time};

/// Monitors a shared process for the interrupt signal.
///
/// If user runs with --monitor or --console, the  Emulator will be running in the foreground,
/// this function listens for the interrupt signal (ctrl+c), once detected, wait for the emulator
/// process to finish then return.
pub fn monitored_child_process(child_arc: &Arc<SharedChild>) -> Result<()> {
    let child_arc_clone = child_arc.clone();
    let term = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&term))?;
    signal_hook::flag::register(signal_hook::consts::SIGHUP, Arc::clone(&term))?;
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&term))?;
    let thread = std::thread::spawn(move || {
        while !term.load(Ordering::Relaxed) && child_arc_clone.try_wait().unwrap().is_none() {
            thread::sleep(time::Duration::from_secs(1));
        }
        child_arc_clone.kill().expect("Error killing for emulator process");
        child_arc_clone.wait().expect("Error waiting for emulator process");
    });
    thread.join().expect("cannot join monitor thread");
    Ok(())
}

/// Returns true if the process identified by the pid is running.
pub fn is_running(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // In strict sandboxed environments (such as public CQ LUCI builders), the `kill`
    // syscall is blocked by seccomp filters even for the process's own PID. This causes
    // `kill(pid, 0)` to fail with EPERM or ENOSYS. Checking `/proc/<pid>` existence is a
    // sandbox-safe alternative on Linux.
    #[cfg(target_os = "linux")]
    {
        if std::path::Path::new(&format!("/proc/{}", pid)).exists() {
            return true;
        }
        if std::path::Path::new("/proc/self").exists() {
            return false;
        }
    }
    // First do a no-hang wait to collect the process if it's defunct.
    let _ = nix::sys::wait::waitpid(
        nix::unistd::Pid::from_raw(pid.try_into().unwrap()),
        Some(nix::sys::wait::WaitPidFlag::WNOHANG),
    );
    // Check to see if it is running by sending signal 0. If there is no error,
    // the process is running.
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid.try_into().unwrap()), None).is_ok()
}

/// Terminates the process.
pub fn terminate(pid: u32) -> Result<()> {
    if pid != 0 && is_running(pid) {
        match nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid.try_into().unwrap()),
            Some(nix::sys::signal::Signal::SIGKILL),
        ) {
            Ok(_) => return Ok(()),
            Err(e) => bail!("Terminate error: {}", e),
        };
    }
    return Ok(());
}
