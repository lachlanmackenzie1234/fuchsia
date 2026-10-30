// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::cc::{Accessory, CcStatus, PowerRole};
use crate::port::{DebounceDirection, TypecPort, TypecState};
use anyhow::Result;
use fidl_next_fuchsia_hardware_usb_connector as fconnector;
use log::debug;
use std::future::Future;

const INSPECTOR_PROP_IS_CONNECTED: &str = "is_connected";
const INSPECTOR_PROP_LAST_UPDATED_TIME_NS: &str = "last_updated_time_ns";
const INSPECTOR_PROP_CC1_STATE: &str = "cc1_state";
const INSPECTOR_PROP_CC2_STATE: &str = "cc2_state";
const INSPECTOR_PROP_TYPEC_STATE: &str = "typec_state";
const INSPECTOR_PROP_ACTIVE_DEBOUNCE_DIRECTION: &str = "active_debounce_direction";
const INSPECTOR_PROP_ATTACH_COUNT: &str = "attach_count";
const INSPECTOR_PROP_DISCONNECT_COUNT: &str = "disconnect_count";
const INSPECTOR_PROP_DEBOUNCE_CANCELLED_COUNT: &str = "debounce_cancelled_count";

/// Hardware driver trait implemented by a TCPC driver to integrate [`TypecPort`].
pub trait Typec: Send + Sync + 'static {
    /// Returns a reference to the driver's [`TypecPort`] state container.
    fn typec(&self) -> &TypecPort;

    /// Reads the instantaneous TCPCI `CC_STATUS` register and returns the decoded [`CcStatus`].
    fn read_cc_status(&self) -> impl Future<Output = Result<CcStatus>> + Send;

    /// Reads whether VBUS is at `vSafe0V` (below 0.8 V). Defaults to `Ok(true)`.
    fn is_vbus_safe0v(&self) -> impl Future<Output = Result<bool>> + Send {
        async { Ok(true) }
    }

    /// Invoked upon entering `AttachWait.SNK` or `AttachWait.SRC`.
    fn on_attach_wait(&self, _role: PowerRole) -> impl Future<Output = ()> + Send {
        async {}
    }

    /// Invoked when the port returns to `Unattached`.
    fn on_unattached(&self) -> impl Future<Output = ()> + Send {
        async {}
    }

    /// Invoked upon completing `tCCDebounce` and transitioning to `Attached.SNK` or `Attached.SRC`.
    fn on_partner_attached(
        &self,
        _role: PowerRole,
        _wake_lease: Option<&zx::EventPair>,
    ) -> impl Future<Output = Result<()>> + Send {
        async { Ok(()) }
    }

    /// Invoked upon completing disconnect debounce and transitioning out of `Attached.*`.
    fn on_partner_detached(
        &self,
        _wake_lease: Option<&zx::EventPair>,
    ) -> impl Future<Output = ()> + Send {
        async {}
    }
}

/// Extension methods on [`Typec`] drivers providing lifecycle controls, state queries, and Inspect
/// telemetry export.
pub trait TypecExt: Typec {
    /// Enables or disables the port.
    ///
    /// When `disabled` is `true`, cancels any uncommitted debounce and sets the state to
    /// [`TypecState::Disabled`]. When `disabled` is `false`, restores the previous attached or
    /// unattached state unless [`Self::reset_to_disconnected`] was called.
    ///
    /// Note: Restoring an uncommitted `Attached.*` state on `set_disabled(false)` is designed for
    /// transactional rollback when a hardware disable write fails. Drivers that re-enable hardware
    /// terminations after a genuine disabled window should call [`Self::reset_to_disconnected`]
    /// and re-sample [`Self::handle_cc_status`].
    fn set_disabled(&self, disabled: bool) {
        self.typec().state.lock().set_disabled(disabled);
    }

    /// Resets the port to disconnected ([`TypecState::Unattached`]; preserved as `Disabled` if the
    /// port is disabled) and cancels any uncommitted debounce without running callbacks.
    fn reset_to_disconnected(&self) {
        self.typec().state.lock().reset_to_disconnected();
    }

    /// Cancels any uncommitted debounce operation without changing the disabled state.
    fn cancel_debounce(&self) {
        self.typec().state.lock().cancel_debounce();
    }

    /// Waits for any in-flight debounce timer or async operation to complete.
    fn wait_for_debounce(&self) -> impl Future<Output = ()> + Send {
        async {}
    }

    /// Returns the current [`TypecState`].
    fn typec_state(&self) -> TypecState {
        self.typec().state.lock().machine.typec_state()
    }

    /// Returns `true` if the port is attached (`Attached.SNK` or `Attached.SRC`).
    fn is_connected(&self) -> bool {
        self.typec_state().is_attached()
    }

    /// Returns the attached [`PowerRole`] if currently in `Attached.SNK` or `Attached.SRC`.
    fn attached_power_role(&self) -> Option<PowerRole> {
        self.typec_state().power_role()
    }

    /// Resolves the Type-C cable plug [`fconnector::Orientation`], or `None` when unattached,
    /// disabled, or unoriented.
    fn orientation(&self) -> Option<fconnector::Orientation> {
        self.typec().state.lock().machine.orientation()
    }

    /// Returns the active [`Accessory`] mode, or `None` when disabled or not in accessory mode.
    fn accessory(&self) -> Option<Accessory> {
        self.typec().state.lock().machine.accessory()
    }

    /// Returns the most recent [`CcStatus`] sample processed by the state machine (`Open` while
    /// disabled).
    fn cc_status(&self) -> CcStatus {
        self.typec().state.lock().machine.visible_cc_status()
    }

    /// Returns `true` if the port's CC terminations are currently disabled.
    fn is_disabled(&self) -> bool {
        self.typec().state.lock().machine.is_disabled()
    }

    /// Read-only hardware query that samples `CC_STATUS` via [`Typec::read_cc_status`] and returns
    /// the instantaneous attachable [`PowerRole`] without mutating [`TypecPort`] state.
    fn sample_attachable_role(&self) -> impl Future<Output = Result<Option<PowerRole>>> + Send {
        async {
            if self.is_disabled() {
                return Ok(None);
            }
            let read_res = self.read_cc_status().await;
            let sink_debug_accessory_mode = {
                let state = self.typec().state.lock();
                if state.machine.is_disabled() {
                    return Ok(None);
                }
                state.machine.sink_debug_accessory_mode
            };
            let cc_status = read_res?;
            if cc_status.looking_for_connection {
                debug!(
                    "Type-C port is searching/DRP toggling (Looking4Connection); treating as disconnected"
                );
            }
            Ok(cc_status.attachable_role(sink_debug_accessory_mode))
        }
    }

    /// Records the port's Type-C state and debounce telemetry properties onto `node`.
    fn record_typec_inspect(&self, node: &fuchsia_inspect::Node) {
        let state = self.typec().state.lock();
        let machine = &state.machine;
        let typec_state = machine.typec_state();
        node.record_bool(INSPECTOR_PROP_IS_CONNECTED, typec_state.is_attached());
        node.record_int(
            INSPECTOR_PROP_LAST_UPDATED_TIME_NS,
            machine.telemetry.last_updated_time.into_nanos(),
        );
        let cc = machine.visible_cc_status();
        node.record_string(INSPECTOR_PROP_CC1_STATE, cc.cc1.as_str());
        node.record_string(INSPECTOR_PROP_CC2_STATE, cc.cc2.as_str());
        node.record_string(INSPECTOR_PROP_TYPEC_STATE, typec_state.as_str());
        let dir_str = state.active_direction().map_or("none", DebounceDirection::as_str);
        node.record_string(INSPECTOR_PROP_ACTIVE_DEBOUNCE_DIRECTION, dir_str);
        node.record_uint(INSPECTOR_PROP_ATTACH_COUNT, machine.telemetry.attach_count);
        node.record_uint(INSPECTOR_PROP_DISCONNECT_COUNT, machine.telemetry.disconnect_count);
        node.record_uint(
            INSPECTOR_PROP_DEBOUNCE_CANCELLED_COUNT,
            machine.telemetry.debounce_cancelled_count,
        );
    }

    /// Returns `true` if a debounce timer or async operation is currently in flight.
    fn is_busy(&self) -> bool {
        self.typec().state.lock().is_busy()
    }

    /// Returns the [`DebounceDirection`] of the currently active debounce task, if any.
    fn active_debounce_direction(&self) -> Option<DebounceDirection> {
        self.typec().state.lock().active_direction()
    }
}

impl<T: Typec> TypecExt for T {}
