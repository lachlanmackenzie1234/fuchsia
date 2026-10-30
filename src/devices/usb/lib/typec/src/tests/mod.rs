// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

mod cc;
mod state;

use crate::port::{
    DEFAULT_CC_ATTACH_DEBOUNCE, DEFAULT_CC_DISCONNECT_DEBOUNCE, DEFAULT_SRC_DISCONNECT_DEBOUNCE,
};
use crate::*;
use anyhow::Result;
use diagnostics_assertions::assert_data_tree;
use fuchsia_async as fasync;
use fuchsia_sync::Mutex;
use std::sync::Arc;

struct FakeTcpc {
    port: TypecPort,
    cc: Mutex<Result<CcStatus, &'static str>>,
}

impl FakeTcpc {
    fn new(scope: &fasync::Scope, initial_cc: CcStatus) -> Arc<Self> {
        Arc::new(Self {
            port: TypecPort::new(scope.to_handle())
                .with_debounce_durations(zx::BootDuration::ZERO, zx::BootDuration::ZERO)
                .with_sink_debug_accessory_mode(true),
            cc: Mutex::new(Ok(initial_cc)),
        })
    }

    fn set_cc(&self, cc: CcStatus) {
        *self.cc.lock() = Ok(cc);
    }
}

impl Typec for FakeTcpc {
    fn typec(&self) -> &TypecPort {
        &self.port
    }

    async fn read_cc_status(&self) -> Result<CcStatus> {
        (*self.cc.lock()).map_err(|msg| anyhow::anyhow!("{msg}"))
    }

    async fn on_partner_attached(
        &self,
        _role: PowerRole,
        _wake_lease: Option<&zx::EventPair>,
    ) -> Result<()> {
        Ok(())
    }

    async fn on_partner_detached(&self, _wake_lease: Option<&zx::EventPair>) {}
}

/// Verifies that `TypecPort` configures default debounce durations, read-only status queries, and
/// default `Typec` trait hooks.
#[fuchsia::test]
async fn test_port_config_and_minimal_trait_impl() {
    let scope = fasync::Scope::new();
    let default_port = TypecPort::new(scope.to_handle());
    assert_eq!(default_port.config.attach_debounce, DEFAULT_CC_ATTACH_DEBOUNCE);
    assert_eq!(default_port.config.disconnect_debounce, DEFAULT_CC_DISCONNECT_DEBOUNCE);
    assert_eq!(default_port.config.source_disconnect_debounce, DEFAULT_SRC_DISCONNECT_DEBOUNCE);
    assert!(!default_port.state.lock().machine.sink_debug_accessory_mode);

    const CUSTOM_SRC_DISCONNECT_DEBOUNCE: zx::BootDuration = zx::BootDuration::from_millis(5);
    let custom_port = TypecPort::new(scope.to_handle())
        .with_debounce_durations(zx::BootDuration::ZERO, zx::BootDuration::ZERO)
        .with_source_disconnect_debounce(CUSTOM_SRC_DISCONNECT_DEBOUNCE)
        .with_sink_debug_accessory_mode(true);
    assert_eq!(custom_port.config.attach_debounce, zx::BootDuration::ZERO);
    assert_eq!(custom_port.config.disconnect_debounce, zx::BootDuration::ZERO);
    assert_eq!(custom_port.config.source_disconnect_debounce, CUSTOM_SRC_DISCONNECT_DEBOUNCE);
    assert!(custom_port.state.lock().machine.sink_debug_accessory_mode);
    assert!(format!("{default_port:?}").contains("TypecPort"));
    {
        // Format from another thread: a same-thread `try_lock` while holding the lock is reported
        // as a lock cycle when `fuchsia_sync` lock-cycle detection is enabled (debug builds).
        let _locked = default_port.state.lock();
        let formatted = std::thread::scope(|s| s.spawn(|| format!("{default_port:?}")).join());
        assert!(formatted.unwrap().contains("<locked>"));
    }

    let snk_dbg = CcStatus::new(CcPinState::Rp3a0, CcPinState::RpDefault);
    let searching = CcStatus::LOOKING_FOR_CONNECTION;
    let tcpc = FakeTcpc::new(&scope, snk_dbg);

    let inspector = fuchsia_inspect::Inspector::default();
    tcpc.record_typec_inspect(inspector.root());
    assert_data_tree!(inspector, root: {
        is_connected: false,
        last_updated_time_ns: 0i64,
        cc1_state: "Open",
        cc2_state: "Open",
        typec_state: "Unattached",
        active_debounce_direction: "none",
        attach_count: 0u64,
        disconnect_count: 0u64,
        debounce_cancelled_count: 0u64,
    });

    assert_eq!(tcpc.cc_status(), CcStatus::default());
    assert!(!tcpc.is_connected());
    assert!(!tcpc.typec_state().is_sourcing_vbus());
    assert_eq!(tcpc.attached_power_role(), None);
    assert_eq!(tcpc.orientation(), None);
    assert_eq!(tcpc.accessory(), None);
    assert!(!tcpc.is_busy());
    assert_eq!(tcpc.active_debounce_direction(), None);
    assert_eq!(tcpc.port.state.lock().machine.telemetry.last_updated_time, zx::BootInstant::ZERO);
    assert_eq!(tcpc.sample_attachable_role().await.unwrap(), Some(PowerRole::Sink));
    assert_eq!(tcpc.cc_status(), CcStatus::default());
    assert_eq!(tcpc.typec_state(), TypecState::Unattached);

    tcpc.set_cc(CcStatus::new(CcPinState::SrcRd, CcPinState::Open));
    assert_eq!(tcpc.sample_attachable_role().await.unwrap(), Some(PowerRole::Source));
    tcpc.set_cc(searching);
    assert_eq!(tcpc.sample_attachable_role().await.unwrap(), None);

    struct MinimalTcpc(TypecPort, CcStatus);
    impl Typec for MinimalTcpc {
        fn typec(&self) -> &TypecPort {
            &self.0
        }
        async fn read_cc_status(&self) -> Result<CcStatus> {
            Ok(self.1)
        }
        async fn on_partner_attached(
            &self,
            _role: PowerRole,
            _wake_lease: Option<&zx::EventPair>,
        ) -> Result<()> {
            Ok(())
        }
        async fn on_partner_detached(&self, _wake_lease: Option<&zx::EventPair>) {}
    }
    let minimal = Arc::new(MinimalTcpc(TypecPort::new(scope.to_handle()), snk_dbg));
    assert_eq!(minimal.sample_attachable_role().await.unwrap(), None);
    minimal.on_attach_wait(PowerRole::Sink).await;
    minimal.on_unattached().await;
    minimal.cancel_debounce();
    minimal.reset_to_disconnected();
    minimal.wait_for_debounce().await;
    minimal.set_disabled(true);
    assert!(minimal.is_disabled());
    assert_eq!(minimal.typec_state(), TypecState::Disabled);
    minimal.set_disabled(false);
    assert!(!minimal.is_disabled());
    assert_eq!(minimal.typec_state(), TypecState::Unattached);
}
