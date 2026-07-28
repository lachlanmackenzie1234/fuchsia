// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::att::bearer::{BearerRecvError, BearerRx};
use crate::att::l2cap::L2CapChannelRx;
use crate::att::pdu::{Opcode, Packet};
use core::fmt;
use core::mem::MaybeUninit;
use sapphire_async::mutex::Mutex as AsyncMutex;
use sapphire_async::notification::Notification;
use sapphire_sync::mutex::Mutex;
use sapphire_sync::mutex::raw::{RawMutex, SingleThreadMutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteFilter {
    Responses,
    Requests,
    Notifications,
}

impl RouteFilter {
    pub fn matches(&self, opcode: Opcode) -> bool {
        match self {
            RouteFilter::Responses => matches!(
                opcode,
                Opcode::ErrorRsp
                    | Opcode::ExchangeMtuRsp
                    | Opcode::FindInformationRsp
                    | Opcode::FindByTypeValueRsp
                    | Opcode::ReadByTypeRsp
                    | Opcode::ReadRsp
                    | Opcode::ReadBlobRsp
                    | Opcode::ReadByGroupTypeRsp
                    | Opcode::WriteRsp
                    | Opcode::PrepareWriteRsp
                    | Opcode::ExecuteWriteRsp
            ),
            RouteFilter::Requests => matches!(
                opcode,
                Opcode::ExchangeMtuReq
                    | Opcode::FindInformationReq
                    | Opcode::FindByTypeValueReq
                    | Opcode::ReadByTypeReq
                    | Opcode::ReadReq
                    | Opcode::ReadBlobReq
                    | Opcode::ReadByGroupTypeReq
                    | Opcode::WriteReq
                    | Opcode::WriteCmd
                    | Opcode::PrepareWriteReq
                    | Opcode::ExecuteWriteReq
            ),
            RouteFilter::Notifications => matches!(opcode, Opcode::HandleValueNtf),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct RouteSet {
    responses: bool,
    requests: bool,
    notifications: bool,
}

impl RouteSet {
    fn claim(&mut self, filter: RouteFilter) -> bool {
        match filter {
            RouteFilter::Responses => {
                if self.responses {
                    false
                } else {
                    self.responses = true;
                    true
                }
            }
            RouteFilter::Requests => {
                if self.requests {
                    false
                } else {
                    self.requests = true;
                    true
                }
            }
            RouteFilter::Notifications => {
                if self.notifications {
                    false
                } else {
                    self.notifications = true;
                    true
                }
            }
        }
    }
}

/// A statically-dispatched, RPC-less demultiplexer that allows a Client, Server, and
/// NotificationStream to safely share and poll a single physical L2CapChannelRx link.
pub struct BearerRouter<Rx, Mtx = SingleThreadMutex> {
    claimed_routes: Mutex<Mtx, RouteSet>,
    bearer_rx: AsyncMutex<Mtx, BearerRx<Rx>>,
    route_notifier: Notification<Mtx>,
}

impl<Rx, Mtx> fmt::Debug for BearerRouter<Rx, Mtx> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BearerRouter").finish_non_exhaustive()
    }
}

impl<Rx: L2CapChannelRx, Mtx: RawMutex> BearerRouter<Rx, Mtx> {
    pub fn new(bearer_rx: Rx) -> Self {
        Self {
            claimed_routes: Mutex::new(RouteSet::default()),
            bearer_rx: AsyncMutex::new(BearerRx::new(bearer_rx)),
            route_notifier: Notification::new(),
        }
    }

    pub fn route_to(&self, filter: RouteFilter) -> Option<BearerRxHandle<'_, Rx, Mtx>> {
        let mut routes = self.claimed_routes.lock();
        routes.claim(filter).then(|| BearerRxHandle::new(self, filter))
    }

    /// Updates the MTU on the shared underlying `BearerRx`.
    pub fn set_mtu(&self, mtu: u16) {
        if let Some(mut bearer) = self.bearer_rx.try_lock() {
            bearer.set_mtu(mtu);
        }
    }
}

#[derive(Debug)]
pub struct BearerRxHandle<'a, Rx, Mtx = SingleThreadMutex> {
    router: &'a BearerRouter<Rx, Mtx>,
    filter: RouteFilter,
}

impl<'a, Rx, Mtx> BearerRxHandle<'a, Rx, Mtx> {
    pub fn new(router: &'a BearerRouter<Rx, Mtx>, filter: RouteFilter) -> Self {
        Self { router, filter }
    }
}

impl<'a, Rx: L2CapChannelRx, Mtx: RawMutex> BearerRxHandle<'a, Rx, Mtx> {
    /// Updates the negotiated ATT MTU boundary on the underlying bearer.
    pub fn set_mtu(&self, mtu: u16) {
        self.router.set_mtu(mtu);
    }

    /// Awaits the next packet matching this handle's `RouteFilter`.
    pub async fn next_packet<'b>(
        &mut self,
        buf: &'b mut [MaybeUninit<u8>],
    ) -> Result<&'b mut Packet, BearerRecvError> {
        let mut lock = self.router.bearer_rx.lock().await;
        while !self.filter.matches(lock.peek_opcode().await) {
            self.router.route_notifier.notify_all();
            lock = {
                self.router.route_notifier.wait_and(move || drop(lock)).await;
                self.router.bearer_rx.lock().await
            };
        }
        let packet_res = lock.next_packet(buf).await;
        drop(lock);
        self.router.route_notifier.notify_all();
        packet_res
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::att::bearer::{BearerTx, MAX_SUPPORTED_MTU};
    use crate::att::client::NotificationStream;
    use crate::att::l2cap::mock::setup_mock_channel;
    use crate::att::pdu::{
        DynamicPacketBuilder, HandleValueNtfHeader, Header, Opcode, PacketBuilder, ReadReq,
    };
    use core::mem::MaybeUninit;
    use sapphire_async::executor::BoundedExecutor;
    use sapphire_async::testing::TestExecutor;
    use zerocopy::{TryFromBytes, U16};

    #[test]
    fn test_router_notifications() {
        let (app_channel, server_tx, _server_rx) = setup_mock_channel();
        let router = BearerRouter::<_>::new(app_channel.receiver);
        let router_ref = &router;
        BoundedExecutor::new(TestExecutor::new(), |executor| {
            let mut bearer_tx = BearerTx::new(server_tx);

            let mut notification_stream =
                NotificationStream::new(router_ref).expect("notification stream created");

            let _sender_handle = executor.spawn(async move {
                for i in 0..4u16 {
                    let header = PacketBuilder {
                        header: Header { opcode: Opcode::HandleValueNtf },
                        payload: HandleValueNtfHeader { attribute_handle: U16::new(i + 1) },
                    };
                    let mut tx_buf = [0u8; 64];
                    let mut builder = DynamicPacketBuilder::<_, u8>::new(&mut tx_buf, header, 64);
                    builder.extend_from_slice(&[0xAA, 0xBB]).unwrap();
                    let _ = bearer_tx.send(builder.as_packet()).await;
                }
                futures::future::pending::<()>().await;
            });

            executor.run_until_stalled();

            let test_listener = executor.spawn(async move {
                let mut rx_buf = [MaybeUninit::uninit(); MAX_SUPPORTED_MTU];

                let ntf1 = notification_stream.next(&mut rx_buf).await.unwrap();
                assert_eq!(ntf1.header.attribute_handle.get(), 1);

                let ntf2 = notification_stream.next(&mut rx_buf).await.unwrap();
                assert_eq!(ntf2.header.attribute_handle.get(), 2);

                let ntf3 = notification_stream.next(&mut rx_buf).await.unwrap();
                assert_eq!(ntf3.header.attribute_handle.get(), 3);

                let ntf4 = notification_stream.next(&mut rx_buf).await.unwrap();
                assert_eq!(ntf4.header.attribute_handle.get(), 4);
            });

            executor.run_until_stalled();
            assert!(test_listener.is_finished());
        });
    }

    #[test]
    fn test_router_buffer_too_small_repush() {
        let (app_channel, server_tx, _server_rx) = setup_mock_channel();
        let router = BearerRouter::<_>::new(app_channel.receiver);
        let router_ref = &router;
        BoundedExecutor::new(TestExecutor::new(), |executor| {
            let mut bearer_tx = BearerTx::new(server_tx);

            let notification_rx_handle = router_ref.route_to(RouteFilter::Notifications).unwrap();

            let _sender_handle = executor.spawn(async move {
                let header = PacketBuilder {
                    header: Header { opcode: Opcode::HandleValueNtf },
                    payload: HandleValueNtfHeader { attribute_handle: U16::new(0x0001) },
                };
                let mut tx_buf = [0u8; 64];
                let mut builder = DynamicPacketBuilder::<_, u8>::new(&mut tx_buf, header, 64);
                builder.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]).unwrap();
                let _ = bearer_tx.send(builder.as_packet()).await;
                futures::future::pending::<()>().await;
            });

            executor.run_until_stalled();

            let test_listener = executor.spawn(async move {
                let mut small_rx_buf = [MaybeUninit::uninit(); 3];
                let mut large_rx_buf = [MaybeUninit::uninit(); 64];

                let mut notification_rx_handle = notification_rx_handle;
                let res = notification_rx_handle.next_packet(&mut small_rx_buf).await;
                assert!(matches!(res, Err(BearerRecvError::BufferTooSmall)));

                let p = notification_rx_handle.next_packet(&mut large_rx_buf).await.unwrap();
                assert_eq!(p.header.opcode, Opcode::HandleValueNtf);
            });

            executor.run_until_stalled();
            assert!(test_listener.is_finished());
        });
    }

    #[test]
    fn test_router_server_request_inbox() {
        let (app_channel, _server_tx, server_rx) = setup_mock_channel();
        let router = BearerRouter::<_>::new(server_rx);
        let router_ref = &router;
        BoundedExecutor::new(TestExecutor::new(), |executor| {
            let mut client_tx_bearer = BearerTx::new(app_channel.sender);

            let server_rx_handle = router_ref.route_to(RouteFilter::Requests).unwrap();

            let sender_handle = executor.spawn(async move {
                let header = PacketBuilder {
                    header: Header { opcode: Opcode::ReadReq },
                    payload: ReadReq { attribute_handle: U16::new(0x0001) },
                };
                let _ = client_tx_bearer.send(header.as_packet()).await;
            });

            let test_server_listener = executor.spawn(async move {
                let mut rx_buf = [MaybeUninit::uninit(); MAX_SUPPORTED_MTU];
                let mut server_rx_handle = server_rx_handle;
                let p = server_rx_handle.next_packet(&mut rx_buf).await.unwrap();
                assert_eq!(p.header.opcode, Opcode::ReadReq);
                let req = ReadReq::try_ref_from_bytes(&p.data).unwrap();
                assert_eq!(req.attribute_handle.get(), 0x0001);
            });

            executor.run_until_stalled();
            assert!(sender_handle.is_finished());
            assert!(test_server_listener.is_finished());
        });
    }

    #[test]
    fn test_router_route_to_duplicate_returns_none() {
        let (app_channel, _server_tx, _server_rx) = setup_mock_channel();
        let router = BearerRouter::<_>::new(app_channel.receiver);
        assert!(router.route_to(RouteFilter::Responses).is_some());
        assert!(router.route_to(RouteFilter::Responses).is_none());
        assert!(router.route_to(RouteFilter::Requests).is_some());
        assert!(router.route_to(RouteFilter::Requests).is_none());
        assert!(router.route_to(RouteFilter::Notifications).is_some());
        assert!(router.route_to(RouteFilter::Notifications).is_none());
    }
}
