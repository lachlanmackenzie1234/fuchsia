// Copyright 2025 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use fidl_fuchsia_net_policy_properties as fnp_properties;
use fidl_fuchsia_net_policy_socketproxy as fnp_socketproxy;
use fuchsia_component::server::ServiceFs;
use futures::stream::StreamExt as _;
use log::debug;

enum IncomingServices {
    NetworkRegistry(fnp_socketproxy::NetworkRegistryRequestStream),
    Networks(fnp_properties::NetworksRequestStream),
}

impl std::fmt::Debug for IncomingServices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NetworkRegistry(_) => f.debug_tuple("NetworkRegistry").finish(),
            Self::Networks(_) => f.debug_tuple("Networks").finish(),
        }
    }
}

#[derive(Debug)]
enum Event {
    NetworkRegistryRequest(Result<fnp_socketproxy::NetworkRegistryRequest, fidl::Error>),
    NetworksAttributesRequest(
        (netcfg::network::ConnectionId, Result<fnp_properties::NetworksRequest, fidl::Error>),
    ),
}

#[fuchsia::main]
async fn main() {
    debug!("Starting fake-netcfg");

    let mut fs = ServiceFs::new_local();
    let _ = fs
        .dir("svc")
        .add_fidl_service(IncomingServices::NetworkRegistry)
        .add_fidl_service(IncomingServices::Networks);
    let _ = fs.take_and_serve_directory_handle().expect("must serve ServiceFs");
    let mut fs = fs.fuse();

    let mut network_registry_streams =
        futures::stream::SelectAll::<fnp_socketproxy::NetworkRegistryRequestStream>::default();
    let mut networks_streams = netcfg::network::ConnectionTagged::default();

    let mut networks_service = netcfg::network::NetpolNetworksService::default();

    loop {
        let event = futures::select! {
            req_stream = fs.select_next_some() => {
                match req_stream {
                    IncomingServices::NetworkRegistry(rs) => network_registry_streams.push(rs),
                    IncomingServices::Networks(rs) => networks_streams.push(rs),
                }
                continue;
            }
            network_registry_req = network_registry_streams.select_next_some() => {
                Event::NetworkRegistryRequest(network_registry_req)
            }
            net_attr_req = networks_streams.select_next_some() => {
                Event::NetworksAttributesRequest(net_attr_req)
            }
        };

        match event {
            Event::NetworkRegistryRequest(req) => {
                let update_result: netcfg::network::DelegatedNetworkUpdateResult = networks_service
                    .handle_delegated_networks_update(req)
                    .await
                    .unwrap_or_else(|e| {
                        log::error!("Could not handle delegated network update: {e:?}");
                        netcfg::network::DelegatedNetworkUpdateResult { dns_servers: None }
                    });
                if let Some(dns_servers) = update_result.dns_servers {
                    networks_service
                        .update(netcfg::network::PropertyUpdate::UpdateDns(dns_servers))
                        .await;
                }
            }
            Event::NetworksAttributesRequest((id, req)) => networks_service
                .handle_network_attributes_request(id, req)
                .await
                .expect("could not handle attribute request"),
        }
    }
}
