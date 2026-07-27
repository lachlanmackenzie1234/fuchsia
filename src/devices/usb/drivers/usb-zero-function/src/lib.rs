// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use fdf_component::{Driver, DriverContext, DriverError, Node, driver_register};
use fuchsia_async as fasync;
use log::info;
use std::sync::Arc;

struct UsbZeroFunction {
    _node: Node,
    _scope: Arc<fasync::Scope>,
}

driver_register!(UsbZeroFunction);

impl Driver for UsbZeroFunction {
    const NAME: &str = "usb-zero-function";

    async fn start(mut context: DriverContext) -> Result<Self, DriverError> {
        let node = context.take_node()?;
        let scope = Arc::new(fasync::Scope::new_with_name("driver"));

        info!("Starting usb-zero-function skeleton");

        Ok(UsbZeroFunction { _node: node, _scope: scope })
    }

    async fn stop(&self) {}
}

#[cfg(test)]
mod tests {
    #[fuchsia::test]
    async fn test_placeholder() {
        assert_eq!(2 + 2, 4);
    }
}
