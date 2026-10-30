// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::cc::{Accessory, CcPinState, CcStatus, PowerRole};
use crate::port::*;
use crate::state::*;
use fidl_next_fuchsia_hardware_usb_connector::Orientation;

/// Verifies that `TypecState` and `DebounceDirection` format labels and report state predicates.
#[fuchsia::test]
fn test_typec_state_predicates() {
    assert!(!TypecState::Unattached.is_attached());
    assert!(!TypecState::AttachWaitSnk.is_attached());
    assert!(TypecState::AttachedSnk.is_attached());
    assert!(!TypecState::AttachWaitSrc.is_attached());
    assert!(TypecState::AttachedSrc.is_attached());
    assert!(!TypecState::Disabled.is_attached());

    assert!(!TypecState::AttachedSnk.is_sourcing_vbus());
    assert!(TypecState::AttachedSrc.is_sourcing_vbus());

    assert_eq!(TypecState::AttachedSnk.power_role(), Some(PowerRole::Sink));
    assert_eq!(TypecState::AttachedSrc.power_role(), Some(PowerRole::Source));
    assert_eq!(TypecState::AttachWaitSnk.power_role(), None);
    assert_eq!(TypecState::Unattached.power_role(), None);

    assert_eq!(DebounceDirection::Attach.as_str(), "attach");
    assert_eq!(DebounceDirection::Disconnect.as_str(), "disconnect");
}

/// Verifies that `Machine` projects `Phase` variants onto `TypecState`, `orientation`, and
/// `accessory`, and settles uncommitted debounce states on `cancel_debounce`, `set_disabled`, and
/// `reset_to_disconnected`.
#[fuchsia::test]
fn test_machine_phase_projections_and_lifecycle_controls() {
    let mut machine = Machine::new();
    let snk_cc1 = CcStatus::new(CcPinState::RpDefault, CcPinState::Open);
    let src_cc2 = CcStatus::new(CcPinState::Open, CcPinState::SrcRd);
    let snk_dbg = CcStatus::new(CcPinState::Rp3a0, CcPinState::RpDefault);

    assert_eq!(machine.orientation(), None);
    assert_eq!(machine.accessory(), None);

    machine.set_phase(Phase::AttachWait { role: PowerRole::Sink, candidate: Some(snk_dbg) });
    assert_eq!(machine.typec_state(), TypecState::AttachWaitSnk);
    assert_eq!(machine.orientation(), Some(Orientation::Normal));
    assert_eq!(machine.accessory(), None);
    machine.sink_debug_accessory_mode = true;
    assert_eq!(machine.accessory(), Some(Accessory::SinkDebug));

    machine.set_phase(Phase::AttachWait { role: PowerRole::Source, candidate: Some(src_cc2) });
    assert_eq!(machine.typec_state(), TypecState::AttachWaitSrc);
    assert_eq!(machine.orientation(), Some(Orientation::Reverse));
    machine.cancel_debounce();
    assert_eq!(machine.phase, Phase::Unattached);
    assert_eq!(machine.typec_state(), TypecState::Unattached);
    assert_eq!(machine.telemetry.debounce_cancelled_count, 1);

    machine.set_phase(Phase::AttachWait { role: PowerRole::Sink, candidate: Some(snk_cc1) });
    machine.reset_to_disconnected();
    assert_eq!(machine.phase, Phase::Unattached);
    assert_eq!(machine.typec_state(), TypecState::Unattached);
    assert_eq!(machine.telemetry.debounce_cancelled_count, 2);

    machine.cc_status = snk_cc1;
    machine.set_phase(Phase::Attached { role: PowerRole::Sink, cc_status: snk_cc1 });
    assert_eq!(machine.typec_state(), TypecState::AttachedSnk);
    assert_eq!(machine.orientation(), Some(Orientation::Normal));
    assert_eq!(machine.visible_cc_status(), snk_cc1);

    assert!(machine.set_disabled(true));
    assert_eq!(machine.typec_state(), TypecState::Disabled);
    assert_eq!(machine.visible_cc_status(), CcStatus::default());
    assert_eq!(machine.orientation(), None);
    assert_eq!(machine.accessory(), None);
    assert!(!machine.set_disabled(true));

    assert!(machine.set_disabled(false));
    assert_eq!(machine.typec_state(), TypecState::AttachedSnk);
    assert!(!machine.set_disabled(false));

    machine.reset_to_disconnected();
    assert_eq!(machine.phase, Phase::Unattached);
    assert_eq!(machine.typec_state(), TypecState::Unattached);
    assert_eq!(machine.telemetry.debounce_cancelled_count, 2);
}
