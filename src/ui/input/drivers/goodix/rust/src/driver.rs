// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use fdf_component::{Driver, DriverContext, DriverError, Node, driver_register};
use log::info;

struct GoodixDriver {
    _node: Node,
}

driver_register!(GoodixDriver);

impl Driver for GoodixDriver {
    const NAME: &str = "goodix";

    async fn start(mut context: DriverContext) -> Result<Self, DriverError> {
        info!("GoodixDriver (Rust Skeleton) started!");
        let _node = context.take_node()?;
        Ok(Self { _node })
    }

    async fn stop(&self) {
        info!("GoodixDriver (Rust Skeleton) stopped!");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_placeholder() {}
}
