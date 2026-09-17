// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

pub mod client;
pub mod context;
pub mod image;
pub mod models;
pub mod tunnel;

pub use client::GceClient;
pub use context::{GceContext, get_serial_endpoint};
pub use discovery::gce_watcher::{GceInstance, GceInstanceData, GceWatcher};
pub use image::{
    BUNDLE_HASH_HEX_LEN, DEFAULT_GCE_DISK_SIZE, compute_bundle_hash, generate_serial_number,
    package_gce_tar_gz,
};
pub use models::{
    Instance, InstanceList, NetworkInterface, Operation, SerialPortOutput, StopResult,
};
pub use tunnel::{GceTunnel, GceTunnelConfig, read_gce_ssh_pubkey, read_gce_ssh_pubkeys};
