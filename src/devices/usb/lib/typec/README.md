# `typec`: USB Type-C CC and Port State Machine

`//src/devices/usb/lib/typec` implements a hardware-agnostic USB Type-C Configuration Channel (CC)
decoder and non-PD port connection state machine per the **USB Type-C Cable and Connector
Specification (Rel 2.4, §4.5.2, Table 4-34, Annex B)** and the **USB Type-C Port Controller
Interface (TCPCI) Specification (Rev 2.0 Ver 1.3, §4.4)**.

## State Machine

```text
                     +------------------------+
                     |        Disabled        |<--- set_disabled(true)
                     |      (FORCE_OPEN)      |
                     +------------------------+
                                 |
                                 | set_disabled(false)
                                 v
                     +------------------------+
                     |       Unattached       |<--- on_unattached() restarts DRP toggling
                     +------------------------+
                       |                    |
      partner Rp       |                    |       partner Rd
      detected         v                    v       detected
            +-----------------+      +-----------------+
            | AttachWait.SNK  |      | AttachWait.SRC  |
            +-----------------+      +-----------------+
                     |                        |
         tCCDebounce |            tCCDebounce |
                     v                        v
            +-----------------+      +-----------------+
            |  Attached.SNK   |      |  Attached.SRC   |
            | (Role: Device)  |      |  (Role: Host)   |
            +-----------------+      +-----------------+
```

## Driver Integration

Embed a `TypecPort` inside your driver struct, implement `Typec`, and import `TypecExt`:

```rust
use anyhow::Result;
use typec::{CcStatus, PowerRole, RoleControl, Typec, TypecExt, TypecPort};

const REG_ROLE_CONTROL: u8 = 0x1a;
const REG_CC_STATUS: u8 = 0x1d;

pub struct MyTcpc {
    typec: TypecPort,
}

impl Typec for MyTcpc {
    fn typec(&self) -> &TypecPort {
        &self.typec
    }

    async fn read_cc_status(&self) -> Result<CcStatus> {
        let role = RoleControl::from(self.read_reg(REG_ROLE_CONTROL).await?);
        let raw_cc = self.read_reg(REG_CC_STATUS).await?;
        Ok(CcStatus::from_tcpci(raw_cc, role))
    }

    async fn on_attach_wait(&self, _role: PowerRole) {
        // Keep VBUS and VCONN disabled while debouncing AttachWait.SRC.
    }

    async fn on_unattached(&self) {
        // Write ROLE_CONTROL and issue COMMAND.Look4Connection.
    }

    async fn on_partner_attached(
        &self,
        _role: PowerRole,
        _wake_lease: Option<&zx::EventPair>,
    ) -> Result<()> {
        // Enable Sink charging / USB device switch, or enable Source 5V OTG VBUS + host mode.
        Ok(())
    }

    async fn on_partner_detached(&self, _wake_lease: Option<&zx::EventPair>) {
        // Tear down USB data path / OTG VBUS.
    }
}
```

| Lifecycle Event | `TypecPort` / `TypecExt` Call |
| :--- | :--- |
| **Construction** | `TypecPort::new(scope)` with optional `.with_sink_debug_accessory_mode(true)` |
| **Port Disable / Enable** | `set_disabled(true)` -> `wait_for_debounce().await` -> `reset_to_disconnected()` before forcing `ROLE_CONTROL` open (`set_disabled(false)` without `reset_to_disconnected()` only rolls back a failed disable); `set_disabled(false)` after re-enabling hardware terminations |
| **Register Reset (`all_reg_rst`)** | `set_disabled(true)` -> `wait_for_debounce().await` -> `reset_to_disconnected()` -> tear down paths, call `set_disabled(false)` (if the port was enabled), and re-apply hardware masks and terminations |
| **State Queries** | `typec_state()`, `is_connected()`, `attached_power_role()`, `orientation()`, `accessory()`, `cc_status()`, `is_disabled()`, `sample_attachable_role()` |
| **Diagnostics** | `self.record_typec_inspect(&inspect_node)` (`is_connected`, `last_updated_time_ns`, `cc1_state`, `cc2_state`, `typec_state`, `active_debounce_direction`, `attach_count`, `disconnect_count`, `debounce_cancelled_count`), plus `is_busy()` and `active_debounce_direction()` |

## Testing

```bash
fx test typec-tests
```
