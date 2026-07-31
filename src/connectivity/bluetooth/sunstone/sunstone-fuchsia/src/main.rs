// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! sunstone-fuchsia contains the main() function for the bt-host component. It is responsible for
//! connecting to the bt-gap component and vendor drivers and initializing and running the
//! Bluetooth Host.

use core::pin::pin;
use fidl::endpoints::Proxy;
use fidl_fuchsia_bluetooth as bt;
use fidl_fuchsia_bluetooth_host as fidl_host;
use fidl_fuchsia_bluetooth_sys as sys;
use fidl_fuchsia_hardware_bluetooth as hardware;
use fuchsia_async as _;
use futures_util::future::FutureExt as _;
use futures_util::select_biased;
use futures_util::stream::StreamExt as _;
use tracing::{info, warn};

#[fuchsia::main]
async fn main() {
    let config = bt_host_config::Config::take_from_startup_handle();
    info!("Starting Rust bt-host (device_path={})", config.device_path);

    let (dev_proxy, server_end) = fidl::endpoints::create_proxy::<hardware::VendorMarker>();
    if let Err(e) = fdio::service_connect(&config.device_path, server_end.into_channel()) {
        warn!("Failed to connect to Vendor protocol at {}: {e:?}", config.device_path);
        return;
    }

    // Connect to fuchsia.bluetooth.host.Receiver
    let receiver =
        match fuchsia_component::client::connect_to_protocol::<fidl_host::ReceiverMarker>() {
            Ok(proxy) => proxy,
            Err(e) => {
                warn!("Failed to connect to Receiver protocol: {e:?}");
                return;
            }
        };

    let (host_client, host_server) = fidl::endpoints::create_endpoints::<fidl_host::HostMarker>();

    if let Err(e) = receiver.add_host(host_client) {
        warn!("Failed to call add_host on Receiver: {e:?}");
        return;
    }

    let mut stream = host_server.into_stream();

    // TODO(https://fxbug.dev/538185448): Read Bluetooth address from HCI device.
    let host_info = sys::HostInfo {
        id: Some(bt::HostId { value: 1 }),
        technology: Some(sys::TechnologyType::DualMode),
        addresses: Some(vec![bt::Address {
            type_: bt::AddressType::Public,
            bytes: [0, 0, 0, 0, 0, 0],
        }]),
        active: Some(true),
        discoverable: Some(false),
        discovering: Some(false),
        ..Default::default()
    };

    let dev_closed = async move {
        let _ = dev_proxy.on_closed().await;
    }
    .fuse();
    let mut dev_closed = pin!(dev_closed);

    let mut watch_state_sent = false;
    #[allow(clippy::collection_is_never_read)]
    let mut _watch_state_responder: Option<fidl_host::HostWatchStateResponder> = None;

    loop {
        let req = select_biased! {
            _ = &mut dev_closed => {
                info!("HCI device node closed; shutting down bt-host");
                break;
            }
            req = stream.next().fuse() => req,
        };

        match req {
            Some(Ok(fidl_host::HostRequest::WatchState { responder })) => {
                if !watch_state_sent {
                    let _ = responder.send(&host_info);
                    watch_state_sent = true;
                } else {
                    // Hanging get: defer response until state changes.
                    _watch_state_responder = Some(responder);
                }
            }
            Some(Ok(fidl_host::HostRequest::Shutdown { .. })) => {
                info!("Received Shutdown request; shutting down bt-host");
                break;
            }
            Some(Ok(_)) => {
                // Ignore unhandled requests for now
            }
            Some(Err(e)) => {
                warn!("Host request stream error: {e:?}");
                break;
            }
            None => {
                info!("Host channel closed by client");
                break;
            }
        }
    }
}
