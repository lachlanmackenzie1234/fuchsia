// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::cc::PowerRole;
use crate::state::Machine;
use fuchsia_async as fasync;
use fuchsia_sync::Mutex;
use std::fmt;

/// Default Type-C attach debounce duration (`tCCDebounce`: 170ms within the 100ms - 200ms window).
pub(crate) const DEFAULT_CC_ATTACH_DEBOUNCE: zx::BootDuration = zx::BootDuration::from_millis(170);

/// Default Type-C Sink disconnect debounce duration (`tPDDebounce`: 20ms within the 10ms - 20ms
/// window).
pub(crate) const DEFAULT_CC_DISCONNECT_DEBOUNCE: zx::BootDuration =
    zx::BootDuration::from_millis(20);

/// Default Type-C Source disconnect debounce duration (`tSRCDisconnect`: 0ms so a Source removes
/// VBUS immediately upon `SRC.Open`).
pub(crate) const DEFAULT_SRC_DISCONNECT_DEBOUNCE: zx::BootDuration = zx::BootDuration::ZERO;

/// Connection state of a Type-C port (USB Type-C Specification Rel 2.4, Section 4.5.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TypecState {
    /// Port is unattached or actively searching (`Looking4Connection`). Covers both
    /// `Unattached.SNK` and `Unattached.SRC` because the TCPC toggles between them in hardware.
    #[default]
    Unattached,
    /// Partner `Rp` detected on CC; debouncing `tCCDebounce` before entering `AttachedSnk` (or
    /// holding an unsupported `Rp/Rp` Debug Accessory when Sink Debug Accessory Mode is disabled).
    AttachWaitSnk,
    /// Attached as a power Sink (`Rd` terminated, partner `Rp` debounced; also used when attached
    /// to an enabled Sink Debug Accessory cable, in which case [`crate::TypecExt::accessory`]
    /// returns [`Some(crate::Accessory::SinkDebug)`]).
    AttachedSnk,
    /// Partner `Rd` detected on CC; debouncing `tCCDebounce` and waiting for `vSafe0V` before
    /// entering `AttachedSrc` (or holding unsupported `Rd/Rd` or `Ra/Ra` terminations with VBUS at
    /// 0 V).
    AttachWaitSrc,
    /// Attached as a power Source (`Rp` terminated, partner `Rd` debounced, `vSafe0V` verified).
    AttachedSrc,
    /// Port CC terminations are disabled (`ROLE_CONTROL = FORCE_OPEN`).
    Disabled,
}

impl TypecState {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Unattached => "Unattached",
            Self::AttachWaitSnk => "AttachWait.SNK",
            Self::AttachedSnk => "Attached.SNK",
            Self::AttachWaitSrc => "AttachWait.SRC",
            Self::AttachedSrc => "Attached.SRC",
            Self::Disabled => "Disabled",
        }
    }

    /// Returns `true` if the port has completed debounce and is in an attached state
    /// (`AttachedSnk` or `AttachedSrc`).
    pub const fn is_attached(self) -> bool {
        matches!(self, Self::AttachedSnk | Self::AttachedSrc)
    }

    /// Returns `true` if the port is in a state that sources VBUS (`AttachedSrc`).
    pub const fn is_sourcing_vbus(self) -> bool {
        matches!(self, Self::AttachedSrc)
    }

    /// Returns the active [`PowerRole`] when the port is in an attached state (`AttachedSnk` or
    /// `AttachedSrc`), or `None` while unattached, disabled, or debouncing.
    pub const fn power_role(self) -> Option<PowerRole> {
        match self {
            Self::AttachedSnk => Some(PowerRole::Sink),
            Self::AttachedSrc => Some(PowerRole::Source),
            Self::Unattached | Self::AttachWaitSnk | Self::AttachWaitSrc | Self::Disabled => None,
        }
    }
}

/// Direction of an active Type-C debounce operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DebounceDirection {
    /// Debouncing a Sink or Source attachment (`tCCDebounce`: 100ms - 200ms).
    Attach,
    /// Debouncing a partner disconnect (`tPDDebounce`: 10ms - 20ms for Sink, `tSRCDisconnect`:
    /// 0ms - 20ms for Source).
    Disconnect,
}

impl DebounceDirection {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Attach => "attach",
            Self::Disconnect => "disconnect",
        }
    }
}

/// Debounce timing of a [`TypecPort`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PortConfig {
    /// `tCCDebounce` applied before `AttachWait.* -> Attached.*`.
    pub(crate) attach_debounce: zx::BootDuration,
    /// `tPDDebounce` applied before `Attached.SNK -> Unattached` and on `AttachWait.SNK` exit.
    pub(crate) disconnect_debounce: zx::BootDuration,
    /// `tSRCDisconnect` applied before `Attached.SRC -> Unattached`.
    pub(crate) source_disconnect_debounce: zx::BootDuration,
}

/// Synchronized state of a [`TypecPort`].
pub(crate) struct PortState {
    pub(crate) machine: Machine,
}

impl PortState {
    pub(crate) const fn new() -> Self {
        Self { machine: Machine::new() }
    }

    /// Returns whether a debounce timer or async operation is currently in flight.
    pub(crate) const fn is_busy(&self) -> bool {
        false
    }

    /// Returns the [`DebounceDirection`] of the debounce currently in flight.
    pub(crate) const fn active_direction(&self) -> Option<DebounceDirection> {
        None
    }

    /// Cancels any uncommitted debounce (`Attached { disconnecting: true }` returns to `Attached`;
    /// `AttachWait*` returns to `Unattached`).
    pub(crate) fn cancel_debounce(&mut self) {
        self.machine.cancel_debounce();
    }

    /// Enables or disables the port and cancels any uncommitted debounce timer.
    pub(crate) fn set_disabled(&mut self, disabled: bool) {
        self.machine.set_disabled(disabled);
    }

    /// Resets the port to disconnected (`disabled` flag preserved) and cancels any uncommitted
    /// debounce timer.
    pub(crate) fn reset_to_disconnected(&mut self) {
        self.machine.reset_to_disconnected();
    }
}

/// USB Type-C Port state container embedded in a driver implementing [`crate::Typec`].
///
/// Holds the synchronized [`crate::CcStatus`], [`TypecState`], and debounce configuration; all
/// state operations are accessed via [`crate::TypecExt`].
pub struct TypecPort {
    /// Scope on which debounce timers and step tasks are spawned.
    _scope: fasync::ScopeHandle,
    pub(crate) config: PortConfig,
    pub(crate) state: Mutex<PortState>,
}

impl fmt::Debug for TypecPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("TypecPort");
        let last_updated_time = if let Some(state) = self.state.try_lock() {
            let typec_state = state.machine.typec_state();
            debug
                .field("cc_status", &state.machine.visible_cc_status())
                .field("typec_state", &typec_state)
                .field("attached_power_role", &typec_state.power_role())
                .field("is_connected", &typec_state.is_attached())
                .field("is_sourcing_vbus", &typec_state.is_sourcing_vbus())
                .field("is_disabled", &state.machine.is_disabled())
                .field("sink_debug_accessory_mode", &state.machine.sink_debug_accessory_mode);
            Some(state.machine.telemetry.last_updated_time)
        } else {
            debug.field("state", &"<locked>");
            None
        };
        debug
            .field("attach_debounce", &self.config.attach_debounce)
            .field("disconnect_debounce", &self.config.disconnect_debounce)
            .field("source_disconnect_debounce", &self.config.source_disconnect_debounce);
        if let Some(last_updated_time) = last_updated_time {
            debug.field("last_updated_time", &last_updated_time);
        }
        debug.finish()
    }
}

impl TypecPort {
    /// Creates a new [`TypecPort`] initialized with standard USB Type-C debounce durations
    /// (`170ms` attach, `20ms` Sink disconnect, `0ms` Source disconnect) and Sink Debug Accessory
    /// Mode disabled by default.
    pub fn new(scope: fasync::ScopeHandle) -> Self {
        Self {
            _scope: scope,
            config: PortConfig {
                attach_debounce: DEFAULT_CC_ATTACH_DEBOUNCE,
                disconnect_debounce: DEFAULT_CC_DISCONNECT_DEBOUNCE,
                source_disconnect_debounce: DEFAULT_SRC_DISCONNECT_DEBOUNCE,
            },
            state: Mutex::new(PortState::new()),
        }
    }

    /// Configures custom `attach_debounce` (`tCCDebounce`: 100ms - 200ms in production) and
    /// `disconnect_debounce` (`tPDDebounce`: 10ms - 20ms in production) durations on this port.
    ///
    /// Out-of-range durations (such as `0ms`) are permitted for deterministic unit testing.
    pub const fn with_debounce_durations(
        mut self,
        attach_debounce: zx::BootDuration,
        disconnect_debounce: zx::BootDuration,
    ) -> Self {
        self.config.attach_debounce = attach_debounce;
        self.config.disconnect_debounce = disconnect_debounce;
        self
    }

    /// Configures a custom `source_disconnect_debounce` (`tSRCDisconnect`: 0ms - 20ms; defaults to
    /// `0ms`) used when detaching from `Attached.SRC`.
    pub const fn with_source_disconnect_debounce(
        mut self,
        source_disconnect_debounce: zx::BootDuration,
    ) -> Self {
        self.config.source_disconnect_debounce = source_disconnect_debounce;
        self
    }

    /// Enables or disables support for Sink Debug Accessory Mode (`Rp/Rp`, e.g. a SuzyQ debug
    /// cable; disabled by default).
    pub fn with_sink_debug_accessory_mode(self, enabled: bool) -> Self {
        self.state.lock().machine.sink_debug_accessory_mode = enabled;
        self
    }
}
