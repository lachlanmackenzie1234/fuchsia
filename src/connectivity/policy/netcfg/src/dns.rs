// Copyright 2020 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use std::collections::HashMap;
use std::pin::Pin;

use fidl_fuchsia_net as fnet;
use fidl_fuchsia_net_name as fnet_name;
use fidl_fuchsia_net_ndp as fnet_ndp;
use fidl_fuchsia_net_ndp_ext as fnet_ndp_ext;

use anyhow::Context;
use async_utils::stream::{Tagged, WithTag as _};
use dns_server_watcher::{DnsServers, DnsServersUpdateSource};
use fidl::endpoints::{ControlHandle as _, Responder as _};
use futures::stream::BoxStream;
use futures::{Stream, StreamExt};
use log::{error, info, trace, warn};
use net_types::{Scope, ScopeableAddress};
use packet_formats::icmp::ndp as packet_formats_ndp;

/// RFC-1035§4.2 specifies port 53 (decimal) as the default port for DNS requests.
pub const DNS_PORT: u16 = 53;

use crate::{DnsWatcherResultPayload, network};

/// Updates the DNS servers used by the DNS resolver.
pub(super) async fn update_servers(
    lookup_admin: &fnet_name::LookupAdminProxy,
    dns_servers: &mut DnsServers,
    dns_server_watch_responders: &mut DnsServerWatchResponders,
    networks_service: &mut network::NetpolNetworksService,
    source: DnsServersUpdateSource,
    servers: Vec<fnet_name::DnsServer_>,
) {
    trace!("updating DNS servers obtained from {:?} to {:?}", source, servers);

    let servers_before = dns_servers.consolidated();
    dns_servers.set_servers_from_source(source, servers);
    let servers = dns_servers.consolidated();
    if servers_before == servers {
        trace!("Update skipped because dns server list has not changed");
        return;
    }
    trace!("updating LookupAdmin with DNS servers = {:?}", servers);

    match lookup_admin.set_dns_servers(&servers).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => warn!("error setting DNS servers: {:?}", zx::Status::from_raw(e)),
        Err(e) => warn!("error sending set DNS servers request: {:?}", e),
    }

    dns_server_watch_responders.send(dns_servers.consolidated_dns_servers());

    networks_service.update(network::PropertyUpdate::dns(dns_servers)).await;
}

/// Creates a stream of RDNSS DNS updates. Returns None if the protocol
/// is not available on the system, indicating the protocol should not be used.
pub(super) async fn create_rdnss_stream(
    watcher_provider: &fnet_ndp::RouterAdvertisementOptionWatcherProviderProxy,
    interface_id: u64,
) -> Option<
    Result<
        impl Stream<Item = Result<(DnsWatcherResultPayload, Vec<fnet_name::DnsServer_>), fidl::Error>>
        + use<>,
        fidl::Error,
    >,
> {
    let watcher_result = fnet_ndp_ext::create_watcher_stream(
        &watcher_provider,
        &fnet_ndp::RouterAdvertisementOptionWatcherParams {
            interest_types: Some(vec![
                packet_formats_ndp::options::NdpOptionType::RecursiveDnsServer.into(),
            ]),
            interest_interface_id: Some(interface_id),
            ..Default::default()
        },
    )
    .await?;

    // This cannot be directly returned using `?` operator since the
    // function returns an Option.
    let watcher = match watcher_result {
        Ok(res) => res,
        Err(e) => return Some(Err(e)),
    };

    Some(Ok(watcher.filter_map(move |entry_res| async move {
        let entry = match entry_res {
            Ok(entry) => entry,
            Err(fnet_ndp_ext::OptionWatchStreamError::Fidl(e)) => {
                return Some(Err(e));
            }
            Err(fnet_ndp_ext::OptionWatchStreamError::Conversion(e)) => {
                // Netstack didn't uphold the invariant to populate the
                // fields for `OptionWatchEntry`.
                error!("Failed to convert OptionWatchStream item: {e:?}");
                return None;
            }
        };
        match entry {
            fnet_ndp_ext::OptionWatchStreamItem::Entry(entry) => {
                let router_address = entry.source_address;
                match entry.try_parse_as_rdnss() {
                    fnet_ndp_ext::TryParseAsOptionResult::Parsed(option) => Some(Ok((
                        DnsWatcherResultPayload::Ndp { router_address },
                        option
                            .iter_addresses()
                            .iter()
                            .map(|addr| fnet_name::DnsServer_ {
                                address: Some(fnet::SocketAddress::Ipv6(fnet::Ipv6SocketAddress {
                                    address: fnet::Ipv6Address { addr: addr.ipv6_bytes() },
                                    port: DNS_PORT,
                                    // Determine whether the address has a zone or not in accordance
                                    // with https://datatracker.ietf.org/doc/html/rfc8106
                                    zone_index: addr
                                        .scope()
                                        .can_have_zone()
                                        .then_some(interface_id)
                                        .unwrap_or_default(),
                                })),
                                source: Some(fnet_name::DnsServerSource::Ndp(
                                    fnet_name::NdpDnsServerSource {
                                        source_interface: Some(interface_id),
                                        ..Default::default()
                                    },
                                )),
                                ..Default::default()
                            })
                            .collect::<Vec<_>>(),
                    ))),
                    fnet_ndp_ext::TryParseAsOptionResult::OptionTypeMismatch => {
                        // Netstack didn't respect our interest configuration.
                        error!("Option type provided did not match RDNSS option type");
                        None
                    }
                    fnet_ndp_ext::TryParseAsOptionResult::ParseErr(err) => {
                        // A network peer could have included an invalid RDNSS option.
                        warn!("Error while parsing as OptionResult: {err:?}");
                        None
                    }
                }
            }
            fnet_ndp_ext::OptionWatchStreamItem::Dropped(num) => {
                warn!(
                    "The server dropped ({num}) NDP options \
                    due to the HangingGet falling behind"
                );
                None
            }
        }
    })))
}

pub(super) async fn add_rdnss_watcher(
    watcher_provider: &fnet_ndp::RouterAdvertisementOptionWatcherProviderProxy,
    interface_id: crate::InterfaceId,
    watchers: &mut crate::DnsServerWatchers<'_>,
) -> Result<(), anyhow::Error> {
    let source = DnsServersUpdateSource::Ndp { interface_id: interface_id.get() };

    // Returns None when RouterAdvertisementOptionWatcherProvider isn't available on the system.
    let stream = create_rdnss_stream(watcher_provider, interface_id.get()).await;

    match stream {
        Some(result) => {
            let tagged_stream = result
                .context("failed to create watcher stream")?
                .tagged(source)
                .map(|(source, res)| match res {
                    Ok((payload, servers)) => (source, payload, Ok(servers)),
                    Err(e) => (source, DnsWatcherResultPayload::Generic, Err(e)),
                })
                .boxed();
            if let Some(o) = watchers.insert(source, tagged_stream) {
                let _: Pin<Box<BoxStream<'_, _>>> = o;
                unreachable!("DNS server watchers must not contain key {:?}", source);
            }
            info!("started NDP watcher on host interface (id={interface_id})");
        }
        None => {
            info!(
                "NDP protocol unavailable: not starting watcher for interface (id={interface_id})"
            );
        }
    }
    Ok(())
}

pub(super) async fn remove_rdnss_watcher(
    lookup_admin: &fnet_name::LookupAdminProxy,
    dns_servers: &mut DnsServers,
    dns_server_watch_responders: &mut DnsServerWatchResponders,
    netpol_networks_service: &mut network::NetpolNetworksService,
    interface_id: crate::InterfaceId,
    watchers: &mut crate::DnsServerWatchers<'_>,
) {
    let source = DnsServersUpdateSource::Ndp { interface_id: interface_id.get() };

    if let None = watchers.remove(&source) {
        // It's surprising that the DNS Watcher for the interface doesn't exist
        // when the RDNSS stream is getting removed, but this can happen
        // when multiple futures try to stop the NDP watcher at the same time.
        warn!(
            "DNS Watcher for key not present; multiple futures stopped NDP \
            watcher for key {:?}; interface_id={}",
            source, interface_id
        );
    }

    update_servers(
        lookup_admin,
        dns_servers,
        dns_server_watch_responders,
        netpol_networks_service,
        source,
        vec![],
    )
    .await;
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ConnectionId(usize);

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct UpdateGeneration(usize);

/// Tracks the currently registered `fnet_name::DnsServerWatcherWatchServersResponder`s.
///
/// Keeps track of which connection ID has been notified for which generation of
/// the DNS server list.
#[derive(Default)]
pub(crate) struct DnsServerWatchResponders {
    /// The current generation. It gets incremented every time the `responders`
    /// list gets emptied by a call to `DnsServerWatchResponders::take`
    generation: UpdateGeneration,

    /// Tracks the last generation for which a DNS server list update has been sent to each client.
    generations: HashMap<ConnectionId, UpdateGeneration>,

    /// The list of registered responders, indexed by their associated client ID.
    responders: HashMap<ConnectionId, fnet_name::DnsServerWatcherWatchServersResponder>,
}

impl DnsServerWatchResponders {
    fn send(&mut self, next_servers: Vec<fnet_name::DnsServer_>) {
        let responders = std::mem::take(&mut self.responders);
        self.generation.0 += 1;
        for (id, responder) in responders {
            match responder.send(&next_servers) {
                Ok(()) => {
                    let _: Option<UpdateGeneration> = self.generations.insert(id, self.generation);
                }
                Err(e) => warn!("Error responding to DnsServerWatcher request: {e:?}"),
            }
        }
    }

    /// Handles a call to `fuchsia.net.name/DnsServerWatcher.WatchServers`, the
    /// responder may be called immediately, or stored for later.
    pub(crate) fn handle_request(
        &mut self,
        id: ConnectionId,
        request: Result<fnet_name::DnsServerWatcherRequest, fidl::Error>,
        servers: &DnsServers,
    ) -> Result<(), fidl::Error> {
        use std::collections::hash_map::Entry;
        match request {
            Ok(fnet_name::DnsServerWatcherRequest::WatchServers { responder }) => {
                match self.responders.entry(id) {
                    Entry::Occupied(_) => {
                        warn!(
                            "Only one call to fuchsia.net.name/DnsServerWatcher.WatchServers \
                            may be active at once"
                        );
                        responder.control_handle().shutdown()
                    }
                    Entry::Vacant(vacant_entry) => {
                        // None is always less than any Some.
                        // See: https://doc.rust-lang.org/std/option/index.html#comparison-operators
                        if self.generations.get(&id) < Some(&self.generation) {
                            let _: Option<_> = self.generations.insert(id, self.generation);
                            responder.send(&servers.consolidated_dns_servers())?;
                        } else {
                            let _: &fnet_name::DnsServerWatcherWatchServersResponder =
                                vacant_entry.insert(responder);
                        }
                    }
                }
            }
            Err(e) => {
                error!("fuchsia.net.name/DnsServerWatcher request error: {:?}", e)
            }
        }

        Ok(())
    }
}

/// Keep track of all of the connected clients of
/// `fuchsia.net.name/DnsServerWatcher` and assign each of them a unique ID.
#[derive(Default)]
pub(crate) struct DnsServerWatcherRequestStreams {
    /// The ID to be assigned to the next connection.
    next_id: ConnectionId,

    /// The currently connected clients.
    request_streams:
        futures::stream::SelectAll<Tagged<ConnectionId, fnet_name::DnsServerWatcherRequestStream>>,
}

impl DnsServerWatcherRequestStreams {
    pub fn handle_request_stream(&mut self, req_stream: fnet_name::DnsServerWatcherRequestStream) {
        self.request_streams.push(req_stream.tagged(self.next_id));
        self.next_id.0 += 1;
    }
}

impl futures::Stream for DnsServerWatcherRequestStreams {
    type Item = (ConnectionId, Result<fnet_name::DnsServerWatcherRequest, fidl::Error>);

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::pin::Pin::new(&mut self.request_streams).poll_next(cx)
    }
}

impl futures::stream::FusedStream for DnsServerWatcherRequestStreams {
    fn is_terminated(&self) -> bool {
        self.request_streams.is_terminated()
    }
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;
    use fuchsia_component::server::{ServiceFs, ServiceFsDir};
    use fuchsia_component_test::{
        Capability, ChildOptions, LocalComponentHandles, RealmBuilder, RealmInstance, Ref, Route,
    };
    use futures::channel::mpsc;
    use futures::{FutureExt as _, SinkExt as _, TryFutureExt as _, TryStreamExt as _};
    use net_declare::{fidl_socket_addr, net_ip_v6};
    use pretty_assertions::assert_eq;

    use super::*;

    enum StubbedServices {
        LookupAdmin(fnet_name::LookupAdminRequestStream),
    }

    async fn run_lookup_admin(handles: LocalComponentHandles) -> Result<(), anyhow::Error> {
        let mut fs = ServiceFs::new();
        let _: &mut ServiceFsDir<'_, _> =
            fs.dir("svc").add_fidl_service(StubbedServices::LookupAdmin);
        let _: &mut ServiceFs<_> = fs.serve_connection(handles.outgoing_dir)?;

        fs.for_each_concurrent(0, move |StubbedServices::LookupAdmin(stream)| async move {
            stream
                .try_for_each(|request| async move {
                    match request {
                        fidl_fuchsia_net_name::LookupAdminRequest::SetDnsServers { .. } => {
                            // Silently ignore this request.
                        }
                        fidl_fuchsia_net_name::LookupAdminRequest::GetDnsServers { .. } => {
                            unimplemented!("Unused in this test")
                        }
                    }
                    Ok(())
                })
                .await
                .context("Failed to serve request stream")
                .unwrap_or_else(|e| warn!("Error encountered: {:?}", e))
        })
        .await;

        Ok(())
    }

    enum IncomingService {
        DnsServerWatcher(fnet_name::DnsServerWatcherRequestStream),
    }

    async fn run_dns_server_watcher(
        handles: LocalComponentHandles,
        mut receiver: mpsc::Receiver<(crate::DnsServersUpdateSource, Vec<fnet_name::DnsServer_>)>,
    ) -> Result<(), anyhow::Error> {
        let connection = handles.connect_to_protocol()?;

        let mut fs = ServiceFs::new();
        let _: &mut ServiceFsDir<'_, _> =
            fs.dir("svc").add_fidl_service(IncomingService::DnsServerWatcher);
        let _: &mut ServiceFs<_> = fs.serve_connection(handles.outgoing_dir)?;

        let mut dns_server_watcher_incoming_requests = DnsServerWatcherRequestStreams::default();
        let mut dns_servers = DnsServers::default();
        let mut dns_server_watch_responders = DnsServerWatchResponders::default();
        let mut netpol_networks_service = network::NetpolNetworksService::default();

        let mut fs = futures::StreamExt::fuse(fs);

        loop {
            futures::select! {
                req_stream = fs.select_next_some() => {
                    match req_stream {
                        IncomingService::DnsServerWatcher(stream) => {
                            dns_server_watcher_incoming_requests.handle_request_stream(stream)
                        }
                    }
                }
                req = dns_server_watcher_incoming_requests.select_next_some() => {
                    let (id, req) = req;
                    dns_server_watch_responders.handle_request(
                        id,
                        req,
                        &dns_servers,
                    )?;
                }
                update = receiver.select_next_some() => {
                    let (source, servers) = update;
                    update_servers(
                        &connection,
                        &mut dns_servers,
                        &mut dns_server_watch_responders,
                        &mut netpol_networks_service,
                        source,
                        servers,
                    ).await
                }
            }
        }
    }

    async fn setup_test() -> Result<
        (RealmInstance, mpsc::Sender<(crate::DnsServersUpdateSource, Vec<fnet_name::DnsServer_>)>),
        anyhow::Error,
    > {
        let (tx, rx) = mpsc::channel(1);
        let builder = RealmBuilder::new().await?;
        let admin_server = builder
            .add_local_child(
                "lookup_admin",
                move |handles: LocalComponentHandles| Box::pin(run_lookup_admin(handles)),
                ChildOptions::new(),
            )
            .await?;

        let dns_server_watcher = builder
            .add_local_child(
                "dns_server_watcher",
                {
                    let rx = fuchsia_sync::Mutex::new(Some(rx));
                    move |handles: LocalComponentHandles| {
                        Box::pin(run_dns_server_watcher(
                            handles,
                            rx.lock()
                                .take()
                                .expect("Only one instance of run_dns_server_watcher should exist"),
                        ))
                    }
                },
                ChildOptions::new(),
            )
            .await?;

        builder
            .add_route(
                Route::new()
                    .capability(Capability::protocol::<fnet_name::DnsServerWatcherMarker>())
                    .from(&dns_server_watcher)
                    .to(Ref::parent()),
            )
            .await?;
        builder
            .add_route(
                Route::new()
                    .capability(Capability::protocol::<fnet_name::LookupAdminMarker>())
                    .from(&admin_server)
                    .to(&dns_server_watcher),
            )
            .await?;

        let realm = builder.build().await?;

        Ok((realm, tx))
    }

    fn server(address: fidl_fuchsia_net::SocketAddress) -> fnet_name::DnsServer_ {
        fnet_name::DnsServer_ { address: Some(address), ..fnet_name::DnsServer_::default() }
    }

    #[fuchsia::test]
    async fn test_dns_server_watcher() -> Result<(), anyhow::Error> {
        let (realm, mut tx) = setup_test().await?;

        let watcher1: fnet_name::DnsServerWatcherProxy = realm
            .root
            .connect_to_protocol_at_exposed_dir()
            .context("While connecting to DnsServerWatcher")?;
        let watcher2: fnet_name::DnsServerWatcherProxy = realm
            .root
            .connect_to_protocol_at_exposed_dir()
            .context("While connecting to DnsServerWatcher")?;

        assert_eq!(watcher1.watch_servers().await?, vec![]);
        assert_eq!(watcher2.watch_servers().await?, vec![]);

        // This next call to watch_servers() should hang, so we expect the on_timeout response.
        let mut watcher1_call = watcher1.watch_servers().fuse();
        futures::select! {
            _ = watcher1_call => {
                return Err(
                    anyhow!("WatchServers should not respond here, there have been no updates")
                );
            },
            _ = fuchsia_async::Timer::new(std::time::Duration::from_millis(100)).fuse() => {}
        }

        // Insert a server from the "Default" source (statically defined).
        let (watch1, watch2, _) = futures::try_join!(
            // This call to watch_servers should now resolve.
            watcher1_call.map_err(|e| anyhow::Error::from(e)),
            watcher2.watch_servers().map_err(|e| anyhow::Error::from(e)),
            tx.send((
                DnsServersUpdateSource::Default,
                vec![server(fidl_socket_addr!("203.0.113.1:1"))],
            ))
            .map_err(|e| anyhow::Error::from(e)),
        )?;
        assert_eq!(watch1, vec![server(fidl_socket_addr!("203.0.113.1:1")),]);
        assert_eq!(watch2, vec![server(fidl_socket_addr!("203.0.113.1:1")),]);

        // Insert a server derived from DHCPv4 interface 1.
        let (watch1, watch2, _) = futures::try_join!(
            watcher1.watch_servers().map_err(|e| anyhow::Error::from(e)),
            watcher2.watch_servers().map_err(|e| anyhow::Error::from(e)),
            tx.send((
                DnsServersUpdateSource::Dhcpv4 { interface_id: 1 },
                vec![server(fidl_socket_addr!("203.0.113.1:2")),],
            ))
            .map_err(|e| anyhow::Error::from(e)),
        )?;
        // The DHCPv4 is expected to be first since the "Default" source is
        // given the lowest priority.
        let expectation = vec![
            server(fidl_socket_addr!("203.0.113.1:2")),
            server(fidl_socket_addr!("203.0.113.1:1")),
        ];
        assert_eq!(watch1, expectation);
        assert_eq!(watch2, expectation);

        // Insert a server derived from DHCPv6 interface 1. Also, only have watcher 1 do the watch.
        let (watch1, _) = futures::try_join!(
            watcher1.watch_servers().map_err(|e| anyhow::Error::from(e)),
            tx.send((
                DnsServersUpdateSource::Dhcpv6 { interface_id: 1 },
                vec![server(fidl_socket_addr!("[2001:db8::]:1")),],
            ))
            .map_err(|e| anyhow::Error::from(e)),
        )?;
        // DHCPv4 is higher priority than DHCPv6, but Default is still the lowest.
        let expectation = vec![
            server(fidl_socket_addr!("203.0.113.1:2")),
            server(fidl_socket_addr!("[2001:db8::]:1")),
            server(fidl_socket_addr!("203.0.113.1:1")),
        ];
        assert_eq!(watch1, expectation);

        // Update the default servers while no watcher is watching. This should
        // increment the generation, meaning that both watchers should respond
        // immediately upon request.
        tx.send((
            DnsServersUpdateSource::Default,
            vec![fnet_name::DnsServer_ {
                address: Some(fidl_socket_addr!("203.0.113.1:5")),
                ..fnet_name::DnsServer_::default()
            }],
        ))
        .await?;
        let (watch1, watch2) = futures::try_join!(
            watcher1.watch_servers().map_err(|e| anyhow::Error::from(e)),
            watcher2.watch_servers().map_err(|e| anyhow::Error::from(e)),
        )?;
        // DHCPv4 is higher priority than DHCPv6, but Default is still the lowest.
        let expectation = vec![
            server(fidl_socket_addr!("203.0.113.1:2")),
            server(fidl_socket_addr!("[2001:db8::]:1")),
            server(fidl_socket_addr!("203.0.113.1:5")),
        ];
        assert_eq!(watch1, expectation);

        // watcher2 has skipped the previous update and just received the most up-to-date.
        assert_eq!(watch2, expectation);

        Ok(())
    }

    #[fuchsia::test]
    async fn test_create_rdnss_stream_router_address_extraction() {
        use futures::StreamExt as _;
        use net_declare::fidl_ip_v6;

        let (provider_proxy, mut provider_stream) = fidl::endpoints::create_proxy_and_stream::<
            fnet_ndp::RouterAdvertisementOptionWatcherProviderMarker,
        >();

        const IFACE_ID: u64 = 1;
        let expected_router_address = net_ip_v6!("fe80::1");

        let stream_fut = async {
            let stream = create_rdnss_stream(&provider_proxy, IFACE_ID)
                .await
                .expect("provider available")
                .expect("stream creation succeeded");
            futures::pin_mut!(stream);
            stream.next().await.expect("stream item available").expect("item ok")
        };

        let server_fut = async {
            let (watcher_server_end, params, _control_handle) = provider_stream
                .next()
                .await
                .expect("provider request")
                .expect("request ok")
                .into_new_router_advertisement_option_watcher()
                .expect("new watcher request");

            assert_eq!(params.interest_interface_id, Some(IFACE_ID));

            let mut watcher_stream = watcher_server_end.into_stream();

            match watcher_stream.next().await.expect("watcher request").expect("request ok") {
                fnet_ndp::OptionWatcherRequest::Probe { responder } => {
                    responder.send().expect("send probe response succeeded");
                }
                req => panic!("expected Probe request, got {req:?}"),
            }

            let responder =
                match watcher_stream.next().await.expect("watcher request").expect("request ok") {
                    fnet_ndp::OptionWatcherRequest::WatchOptions { responder } => responder,
                    req => panic!("expected WatchOptions request, got {req:?}"),
                };

            let mut rdnss_body = vec![
                0, 0, // Reserved (2 bytes)
                0, 0, 0, 120, // Lifetime 120 (4 bytes)
            ];
            rdnss_body
                .extend_from_slice(&[0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);

            let entry = fnet_ndp::OptionWatchEntry {
                interface_id: Some(IFACE_ID),
                source_address: Some(fnet::Ipv6Address {
                    addr: expected_router_address.ipv6_bytes(),
                }),
                option_type: Some(25),
                body: Some(rdnss_body),
                ..Default::default()
            };

            responder.send(&[entry], 0).expect("send response succeeded");
        };

        let ((payload, servers), ()) = futures::join!(stream_fut, server_fut);

        assert_eq!(
            payload,
            DnsWatcherResultPayload::Ndp { router_address: expected_router_address }
        );
        assert_eq!(servers.len(), 1);
        assert_eq!(
            servers[0].address,
            Some(fnet::SocketAddress::Ipv6(fnet::Ipv6SocketAddress {
                address: fidl_ip_v6!("2001:db8::1"),
                port: DNS_PORT,
                zone_index: 0,
            }))
        );
    }
}
