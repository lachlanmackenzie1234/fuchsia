// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! Test root driver for driver-lab driver realm tests.
//!
//! Adds one child node marked as a proxy target and offers it a fake
//! platform device with a single VMO-backed MMIO whose contents are
//! seeded with known values, so the proxy under test binds, maps real
//! (VMO) MMIO, and serves reads the test can verify end to end.

use fake_pdev::FakePDev;
use fdf_component::{Driver, DriverContext, DriverError, Node, NodeBuilder, driver_register};
use fidl_next_fuchsia_hardware_platform_device as fdevice;
use fuchsia_async as fasync;
use fuchsia_component::server::ServiceFs;
use futures::StreamExt;
use log::info;

/// Size of the fake MMIO region.
pub const MMIO_SIZE: u64 = 0x1000;
/// Seeded value at [`SEED_OFFSET`].
pub const SEED_VALUE: u32 = 0xFEED_FACE;
/// Offset of the seeded value.
pub const SEED_OFFSET: u64 = 0x10;

struct LabRootDriver {
    _node: Node,
    _scope: fasync::Scope,
}

driver_register!(LabRootDriver);

fn make_mmio_vmo() -> Result<zx::Vmo, zx::Status> {
    let vmo = zx::Vmo::create(MMIO_SIZE)?;
    // The proxy maps this VMO with device cache policy; set it before
    // any pages are touched, then seed through a mapping (uncached VMOs
    // reject zx_vmo_write).
    vmo.set_cache_policy(zx::CachePolicy::UnCachedDevice)?;
    let mapping = mapped_vmo::Mapping::create_from_vmo(
        &vmo,
        MMIO_SIZE as usize,
        zx::VmarFlags::PERM_READ | zx::VmarFlags::PERM_WRITE,
    )?;
    mapping.write_at(SEED_OFFSET as usize, &SEED_VALUE.to_le_bytes());
    Ok(vmo)
}

impl Driver for LabRootDriver {
    const NAME: &str = "lab_root";

    async fn start(mut context: DriverContext) -> Result<Self, DriverError> {
        let node = context.take_node()?;

        let vmo = make_mmio_vmo().map_err(DriverError::Status)?;
        let mut config = fake_pdev::Config::default();
        config.mmios.insert(
            0,
            fdevice::natural::Mmio { offset: Some(0), size: Some(MMIO_SIZE), vmo: Some(vmo) },
        );
        config.device_info =
            Some(fdevice::natural::NodeDeviceInfo { mmio_count: Some(1), ..Default::default() });
        let pdev = FakePDev::new();
        pdev.set_config(config);

        let scope = fasync::Scope::new_with_name(Self::NAME);
        let mut fs = ServiceFs::new();
        let offer = pdev.serve(&mut fs, scope.to_handle(), "pdev");

        let child = NodeBuilder::new("proxy-target")
            .add_property(bind_fuchsia_driver_lab::PROXY_TARGET, true)
            .add_offer(offer)
            .build();
        node.add_child(child).await?;

        context.serve_outgoing(&mut fs)?;
        scope.spawn(async move {
            fs.collect::<()>().await;
        });

        info!("lab_root added proxy-target node with one fake MMIO");
        Ok(Self { _node: node, _scope: scope })
    }

    async fn stop(&self) {}
}
