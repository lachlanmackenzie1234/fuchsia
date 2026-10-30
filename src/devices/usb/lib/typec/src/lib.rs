// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! Hardware-agnostic USB Type-C Port Manager (TCPM) CC decoding and port state machine.
//!
//! | TCPC driver's role | Item |
//! | :-- | :-- |
//! | Implements | [`Typec`]: register access and partner callbacks |
//! | Embeds | [`TypecPort`]: port configuration and synchronized state |
//! | Calls | [`TypecExt`]: events, port control, queries, and diagnostics |
//! | Decodes | [`CcStatus`] and [`RoleControl`] (TCPCI `CC_STATUS` / `ROLE_CONTROL`) |

mod cc;
mod port;
mod state;
mod typec;

pub use cc::{
    Accessory, COMMAND_LOOK4CONNECTION, CcPinState, CcStatus, CcTermination, PowerRole,
    RoleControl, RpValue,
};
pub use port::{DebounceDirection, TypecPort, TypecState};
pub use typec::{Typec, TypecExt};

#[cfg(test)]
mod tests;
