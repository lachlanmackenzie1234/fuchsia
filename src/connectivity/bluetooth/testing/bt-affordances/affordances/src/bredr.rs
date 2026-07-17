// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::proxies::Proxies;
use anyhow::anyhow;
use fidl_fuchsia_bluetooth::PeerId;
use fidl_fuchsia_bluetooth_bredr::{ConnectParameters, L2capParameters};
use fuchsia_bluetooth::types::Channel;

pub(crate) async fn connect_l2cap(
    proxies: &Proxies,
    peer_id: &PeerId,
    psm: u16,
) -> Result<Channel, anyhow::Error> {
    match proxies
        .profile_proxy
        .connect(
            peer_id,
            &ConnectParameters::L2cap(L2capParameters { psm: Some(psm), ..Default::default() }),
        )
        .await
    {
        Ok(Ok(channel_res)) => Ok(channel_res
            .try_into()
            .map_err(|err| anyhow!("Couldn't convert FIDL to BT channel: {err:?}"))?),
        Ok(Err(sapphire_err)) => {
            Err(anyhow!("fuchsia.bluetooth.bredr.Profile/Connect error: {sapphire_err:?}"))
        }
        Err(fidl_err) => Err(anyhow!("fuchsia.bluetooth.bredr.Profile/Connect error: {fidl_err}")),
    }
}
