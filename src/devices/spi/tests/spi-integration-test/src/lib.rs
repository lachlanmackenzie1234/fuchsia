// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use anyhow::{Context, Result};
use fidl::endpoints::ServiceMarker;
use fidl_fuchsia_hardware_spi as fspi;
use fuchsia_component::client::{connect_to_service_instance, open_service_at};
use fuchsia_fs::directory::{WatchEvent, Watcher};
use futures::StreamExt;

/// Connects to the first available SPI device using TestService.
async fn connect_to_device() -> Result<fspi::DeviceProxy> {
    let service_directory = open_service_at(fspi::TestServiceMarker::SERVICE_NAME)
        .context("Failed to open service directory")?;
    let mut watcher = Watcher::new(&service_directory).await.context("Failed to create watcher")?;

    while let Some(message) = watcher.next().await {
        let message = message.context("Watcher error")?;
        if message.event == WatchEvent::ADD_FILE || message.event == WatchEvent::EXISTING {
            let filename = message.filename.to_str().ok_or_else(|| {
                anyhow::anyhow!("Invalid UTF-8 in filename: {:?}", message.filename)
            })?;
            if filename != "." && filename != ".." {
                let service_proxy =
                    connect_to_service_instance::<fspi::TestServiceMarker>(filename)
                        .context("Failed to connect to TestService instance")?;
                let test_proxy = service_proxy
                    .connect_to_test()
                    .context("Failed to connect to test protocol")?;

                let (device_client, device_server) =
                    fidl::endpoints::create_proxy::<fspi::DeviceMarker>();
                test_proxy
                    .connect_spi_loopback(device_server)
                    .await
                    .context("ConnectSpiLoopback FIDL call failed")?
                    .map_err(|status| anyhow::anyhow!("ConnectSpiLoopback failed: {:?}", status))?;

                return Ok(device_client);
            }
        }
    }
    anyhow::bail!("Watcher ended without finding instance");
}

#[fuchsia::test]
async fn test_can_assert_cs() -> Result<()> {
    let device = connect_to_device().await?;
    let can = device.can_assert_cs().await.context("CanAssertCs FIDL call failed")?;
    println!("CanAssertCs returned: {:?}", can);
    Ok(())
}
