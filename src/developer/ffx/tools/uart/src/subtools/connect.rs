// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::args::ConnectCommand;
use async_trait::async_trait;
use ffx_config::EnvironmentContext;
use ffx_writer::SimpleWriter;
use fho::{FfxMain, FfxTool, Result, user_error};

#[derive(FfxTool)]
#[target(None)]
pub struct ConnectTool {
    #[command]
    pub(crate) cmd: ConnectCommand,
    pub(crate) context: EnvironmentContext,
}

#[async_trait(?Send)]
impl FfxMain for ConnectTool {
    type Writer = SimpleWriter;
    type Error = fho::Error;

    async fn main(self, _writer: Self::Writer) -> Result<()> {
        let _ = self.cmd;
        let spec = crate::get_spec(&self.context, true).await?;
        let _target = crate::resolve_target_to_uart_path(&self.context, &spec).await?;
        Err(user_error!("Connect subcommand implementation will be introduced in follow-up CL."))
    }
}
