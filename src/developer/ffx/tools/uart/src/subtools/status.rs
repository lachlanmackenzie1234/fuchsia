// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::args::StatusCommand;
use async_trait::async_trait;
use ffx_config::EnvironmentContext;
use ffx_writer::SimpleWriter;
use fho::{FfxMain, FfxTool, Result, user_error};

#[derive(FfxTool)]
#[target(None)]
pub struct StatusTool {
    #[command]
    pub(crate) cmd: StatusCommand,
    pub(crate) context: EnvironmentContext,
}

#[async_trait(?Send)]
impl FfxMain for StatusTool {
    type Writer = SimpleWriter;
    type Error = fho::Error;

    async fn main(self, _writer: Self::Writer) -> Result<()> {
        let _ = (self.cmd, self.context);
        Err(user_error!("Command not yet implemented."))
    }
}
