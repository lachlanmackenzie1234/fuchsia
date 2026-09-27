// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use fidl_next;
use fidl_next::fuchsia::zx;
use fidl_next_fuchsia_input_report as fidl_input_report;
use std::sync::Arc;

/// A fake implementation of [`fidl_input_report::InputDevice`].
///
/// This test double supports returning a preset [`fidl_input_report::DeviceDescriptor`].
/// All other API methods return [`zx::Status::NOT_SUPPORTED`] or are unimplemented.
#[derive(Clone)]
pub struct FakeInputDevice {
    inner: Arc<FakeInputDeviceInner>,
}

struct FakeInputDeviceInner {
    descriptor: fidl_input_report::DeviceDescriptor,
}

impl FakeInputDevice {
    /// Creates a new [`FakeInputDevice`] that returns the given
    /// [`fidl_input_report::DeviceInformation`] in its device descriptor.
    pub fn new(device_info: fidl_input_report::DeviceInformation) -> Self {
        Self {
            inner: Arc::new(FakeInputDeviceInner {
                descriptor: fidl_input_report::DeviceDescriptor {
                    device_information: Some(device_info),
                    ..Default::default()
                },
            }),
        }
    }

    /// Serves this [`FakeInputDevice`] on the given [`fuchsia_async::Scope`].
    pub fn serve_on_scope(
        &self,
        scope: &fuchsia_async::Scope,
        server_end: fidl_next::ServerEnd<fidl_input_report::InputDevice>,
    ) {
        let handler = FakeInputDeviceHandler { inner: Arc::clone(&self.inner) };
        scope.spawn(async move {
            let dispatcher =
                fidl_next::ServerDispatcher::<fidl_input_report::InputDevice>::new(server_end);
            if let Err(err) = dispatcher.run(handler).await {
                match err {
                    fidl_next::ProtocolError::PeerClosed | fidl_next::ProtocolError::Stopped => {}
                    _ => log::error!("FakeInputDevice server dispatcher failed: {err:?}"),
                }
            }
        });
    }

    /// Serves this [`FakeInputDevice`] as a [`vfs::service::Service`] node.
    pub fn serve(&self) -> Arc<vfs::service::Service> {
        let fake_device = self.clone();
        vfs::service::endpoint(move |vfs_scope, channel| {
            let server_end = fidl_next::ServerEnd::<fidl_input_report::InputDevice>::from_untyped(
                channel.into_zx_channel(),
            );
            let scope = fuchsia_async::Scope::new();
            fake_device.serve_on_scope(&scope, server_end);
            vfs_scope.spawn(async move {
                scope.await;
            });
        })
    }
}

struct FakeInputDeviceHandler {
    inner: Arc<FakeInputDeviceInner>,
}

impl fidl_input_report::InputDeviceServerHandler for FakeInputDeviceHandler {
    async fn get_descriptor(
        &mut self,
        responder: fidl_next::Responder<fidl_input_report::input_device::GetDescriptor>,
    ) {
        if let Err(err) = responder.respond(&self.inner.descriptor).await {
            log::error!("GetDescriptor responder failed: {err:?}");
        }
    }

    async fn get_input_reports_reader(
        &mut self,
        _request: fidl_next::Request<fidl_input_report::input_device::GetInputReportsReader>,
    ) {
        unimplemented!("GetInputReportsReader (v1) is not supported; use GetInputReportsReaderV2");
    }

    async fn get_input_reports_reader_v2(
        &mut self,
        _request: fidl_next::Request<fidl_input_report::input_device::GetInputReportsReaderV2>,
        _responder: fidl_next::Responder<fidl_input_report::input_device::GetInputReportsReaderV2>,
    ) {
        unimplemented!("GetInputReportsReaderV2 (v2) is not implemented");
    }

    async fn send_output_report(
        &mut self,
        _request: fidl_next::Request<fidl_input_report::input_device::SendOutputReport>,
        responder: fidl_next::Responder<fidl_input_report::input_device::SendOutputReport>,
    ) {
        if let Err(err) = responder.respond_err(zx::Status::NOT_SUPPORTED).await {
            log::error!("SendOutputReport responder failed: {err:?}");
        }
    }

    async fn get_feature_report(
        &mut self,
        responder: fidl_next::Responder<fidl_input_report::input_device::GetFeatureReport>,
    ) {
        if let Err(err) = responder.respond_err(zx::Status::NOT_SUPPORTED).await {
            log::error!("GetFeatureReport responder failed: {err:?}");
        }
    }

    async fn set_feature_report(
        &mut self,
        _request: fidl_next::Request<fidl_input_report::input_device::SetFeatureReport>,
        responder: fidl_next::Responder<fidl_input_report::input_device::SetFeatureReport>,
    ) {
        if let Err(err) = responder.respond_err(zx::Status::NOT_SUPPORTED).await {
            log::error!("SetFeatureReport responder failed: {err:?}");
        }
    }

    async fn get_input_report(
        &mut self,
        _request: fidl_next::Request<fidl_input_report::input_device::GetInputReport>,
        responder: fidl_next::Responder<fidl_input_report::input_device::GetInputReport>,
    ) {
        if let Err(err) = responder.respond_err(zx::Status::NOT_SUPPORTED).await {
            log::error!("GetInputReport responder failed: {err:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use googletest::prelude::*;

    #[gtest]
    #[fuchsia::test]
    async fn test_fake_input_device_descriptor() {
        let fake_device = FakeInputDevice::new(fidl_input_report::DeviceInformation {
            vendor_id: Some(0x1234),
            product_id: Some(0x5678),
            manufacturer_name: Some("TestManuf".to_string()),
            product_name: Some("TestProd".to_string()),
            serial_number: Some("TestSerial".to_string()),
            ..Default::default()
        });
        let scope = fuchsia_async::Scope::new();
        let (client_end, server_end) =
            fidl_next::fuchsia::create_channel::<fidl_input_report::InputDevice>();
        fake_device.serve_on_scope(&scope, server_end);
        let client = client_end.spawn();

        let response = client.get_descriptor().await.expect("get_descriptor failed");
        let info =
            response.descriptor.device_information.as_ref().expect("device_information missing");
        expect_eq!(info.vendor_id, Some(0x1234));
        expect_eq!(info.product_id, Some(0x5678));
        expect_eq!(info.manufacturer_name.as_deref(), Some("TestManuf"));
        expect_eq!(info.product_name.as_deref(), Some("TestProd"));
        expect_eq!(info.serial_number.as_deref(), Some("TestSerial"));
    }

    #[gtest]
    #[fuchsia::test]
    #[should_panic(
        expected = "GetInputReportsReader (v1) is not supported; use GetInputReportsReaderV2"
    )]
    async fn test_fake_input_device_get_input_reports_reader_v1_panics() {
        let fake_device = FakeInputDevice::new(fidl_input_report::DeviceInformation::default());
        let scope = fuchsia_async::Scope::new();
        let (client_end, server_end) =
            fidl_next::fuchsia::create_channel::<fidl_input_report::InputDevice>();
        fake_device.serve_on_scope(&scope, server_end);
        let client = client_end.spawn();

        let (_reader_client_end, reader_server_end) =
            fidl_next::fuchsia::create_channel::<fidl_input_report::InputReportsReader>();
        client.get_input_reports_reader(reader_server_end).await.expect("send failed");
        fuchsia_async::yield_now().await;
        fuchsia_async::Timer::new(fuchsia_async::MonotonicInstant::after(
            fuchsia_async::MonotonicDuration::from_millis(50),
        ))
        .await;
    }

    #[gtest]
    #[fuchsia::test]
    async fn test_fake_input_device_unsupported_methods() {
        let fake_device = FakeInputDevice::new(fidl_input_report::DeviceInformation::default());
        let scope = fuchsia_async::Scope::new();
        let (client_end, server_end) =
            fidl_next::fuchsia::create_channel::<fidl_input_report::InputDevice>();
        fake_device.serve_on_scope(&scope, server_end);
        let client = client_end.spawn();

        let res = client
            .send_output_report(fidl_input_report::OutputReport::default())
            .await
            .expect("fidl call failed");
        expect_eq!(res, Err(Err(zx::Status::NOT_SUPPORTED)));

        let res = client.get_feature_report().await.expect("fidl call failed");
        expect_eq!(res, Err(Err(zx::Status::NOT_SUPPORTED)));

        let res = client
            .set_feature_report(fidl_input_report::FeatureReport::default())
            .await
            .expect("fidl call failed");
        expect_eq!(res, Err(Err(zx::Status::NOT_SUPPORTED)));

        let res = client
            .get_input_report(fidl_input_report::DeviceType::Mouse)
            .await
            .expect("fidl call failed");
        expect_eq!(res, Err(Err(zx::Status::NOT_SUPPORTED)));
    }
}
