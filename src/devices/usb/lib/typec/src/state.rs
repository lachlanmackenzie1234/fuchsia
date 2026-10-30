// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! Pure synchronous USB Type-C connection state machine (USB Type-C Rel 2.4, Section 4.5.2).

use crate::cc::{Accessory, CcStatus, PowerRole};
use crate::port::TypecState;
use fidl_next_fuchsia_hardware_usb_connector as fconnector;

/// Inspect-only telemetry: transitions update it but never branch on it.
pub(crate) struct Telemetry {
    /// Time of the last [`TypecState`] change.
    pub(crate) last_updated_time: zx::BootInstant,
    /// Committed `Attached.*` transitions.
    pub(crate) attach_count: u64,
    /// Confirmed `Attached.* -> Unattached` transitions.
    pub(crate) disconnect_count: u64,
    /// Debounce timers cancelled or superseded before committing.
    pub(crate) debounce_cancelled_count: u64,
}

impl Telemetry {
    const fn new() -> Self {
        Self {
            last_updated_time: zx::BootInstant::ZERO,
            attach_count: 0,
            disconnect_count: 0,
            debounce_cancelled_count: 0,
        }
    }
}

/// Connection phase of [`Machine`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Phase {
    #[default]
    Unattached,
    /// `AttachWait.{SNK,SRC}`: `candidate` is `Some` while debouncing `tCCDebounce`, or `None`
    /// while holding an unsupported accessory in `AttachWait` without a timer.
    AttachWait {
        role: PowerRole,
        candidate: Option<CcStatus>,
    },
    /// `Attached.{SNK,SRC}` after `tCCDebounce` has completed.
    Attached {
        role: PowerRole,
        cc_status: CcStatus,
    },
}

/// Pure USB Type-C connection state machine (USB Type-C Rel 2.4, Section 4.5.2).
pub(crate) struct Machine {
    /// Most recent `CC_STATUS` sample; use [`Self::visible_cc_status`] to report `Open` while
    /// disabled.
    pub(crate) cc_status: CcStatus,
    /// Whether port CC terminations are currently disabled.
    pub(crate) disabled: bool,
    /// Current connection lifecycle phase; [`Self::typec_state`] projects it onto [`TypecState`].
    pub(crate) phase: Phase,
    /// Whether an `Rp/Rp` Sink Debug Accessory (e.g. SuzyQ) may enter `Attached.SNK`.
    pub(crate) sink_debug_accessory_mode: bool,
    /// Inspect-only counters and timestamp.
    pub(crate) telemetry: Telemetry,
}

impl Machine {
    pub(crate) const fn new() -> Self {
        Self {
            cc_status: CcStatus::new(crate::cc::CcPinState::Open, crate::cc::CcPinState::Open),
            disabled: false,
            phase: Phase::Unattached,
            sink_debug_accessory_mode: false,
            telemetry: Telemetry::new(),
        }
    }

    /// Projects current phase and disabled state onto the public [`TypecState`].
    pub(crate) const fn typec_state(&self) -> TypecState {
        if self.disabled {
            return TypecState::Disabled;
        }
        match self.phase {
            Phase::Unattached => TypecState::Unattached,
            Phase::AttachWait { role: PowerRole::Sink, .. } => TypecState::AttachWaitSnk,
            Phase::AttachWait { role: PowerRole::Source, .. } => TypecState::AttachWaitSrc,
            Phase::Attached { role: PowerRole::Sink, .. } => TypecState::AttachedSnk,
            Phase::Attached { role: PowerRole::Source, .. } => TypecState::AttachedSrc,
        }
    }

    pub(crate) const fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// Returns the externally visible `CC_STATUS` (`Open` while disabled).
    pub(crate) fn visible_cc_status(&self) -> CcStatus {
        if self.disabled { CcStatus::default() } else { self.cc_status }
    }

    pub(crate) fn set_phase(&mut self, next_phase: Phase) {
        let prev_state = self.typec_state();
        self.phase = next_phase;
        if self.typec_state() != prev_state {
            self.telemetry.last_updated_time = zx::BootInstant::get();
        }
    }

    /// Returns the `CC_STATUS` that drives [`Self::orientation`] and [`Self::accessory`].
    fn active_cc(&self) -> Option<CcStatus> {
        if self.disabled {
            return None;
        }
        match self.phase {
            Phase::Attached { cc_status, .. } => Some(cc_status),
            Phase::AttachWait { candidate, .. } => candidate,
            Phase::Unattached => None,
        }
    }

    /// Plug orientation of the committed attachment or debounce candidate.
    pub(crate) fn orientation(&self) -> Option<fconnector::Orientation> {
        self.active_cc()?.orientation()
    }

    /// Supported accessory of the committed attachment or debounce candidate.
    pub(crate) fn accessory(&self) -> Option<Accessory> {
        match self.active_cc()?.accessory() {
            Some(Accessory::SinkDebug) if self.sink_debug_accessory_mode => {
                Some(Accessory::SinkDebug)
            }
            Some(Accessory::SinkDebug | Accessory::SourceDebug | Accessory::Audio) | None => None,
        }
    }

    /// Settles `phase` when cancelling an uncommitted debounce (`AttachWait` returns to
    /// `Unattached`).
    pub(crate) fn cancel_debounce(&mut self) {
        match self.phase {
            Phase::Attached { .. } | Phase::Unattached => {}
            Phase::AttachWait { candidate, .. } => {
                if candidate.is_some() {
                    self.telemetry.debounce_cancelled_count += 1;
                }
                self.cc_status = CcStatus::default();
                self.set_phase(Phase::Unattached);
            }
        }
    }

    /// Updates disabled state, returning `true` when changed.
    pub(crate) fn set_disabled(&mut self, disabled: bool) -> bool {
        if self.disabled == disabled {
            return false;
        }
        if disabled {
            self.cancel_debounce();
        }
        self.disabled = disabled;
        self.telemetry.last_updated_time = zx::BootInstant::get();
        true
    }

    /// Resets the port to disconnected (`disabled` flag preserved) and cancels any uncommitted
    /// debounce.
    pub(crate) fn reset_to_disconnected(&mut self) {
        self.cancel_debounce();
        self.phase = Phase::Unattached;
        self.cc_status = CcStatus::default();
        if !self.disabled {
            self.telemetry.last_updated_time = zx::BootInstant::get();
        }
    }
}
