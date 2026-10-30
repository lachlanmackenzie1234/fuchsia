// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#![recursion_limit = "256"]

use starnix_core::device::kobject::{Device, UEventAction};
use starnix_core::fs::sysfs::build_device_directory;
use starnix_core::task::{CurrentTask, Kernel};
use starnix_core::vfs::FsNodeOps;
use starnix_core::vfs::pseudo::simple_directory::SimpleDirectoryMutator;
use starnix_core::vfs::pseudo::simple_file::{BytesFile, BytesFileOps, serialize_for_file};
use starnix_sync::{LockDepRwLock, PowerSupplyStateLock};
use starnix_uapi::errors::Errno;
use starnix_uapi::file_mode::mode;
use starnix_uapi::{errno, error};
use std::borrow::Cow;
use std::fmt::Display;
use std::sync::Arc;

/// Default maximum charger voltage in microvolts (5V).
const DEFAULT_CHARGER_MAX_VOLTAGE_UV: i32 = 5_000_000;

/// Linux `power_supply` `usb_type` strings.
const USB_TYPE_UNKNOWN: &str = "[Unknown] SDP DCP CDP";

/// In-memory state of the `/sys/class/power_supply/battery` device.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct BatteryState {
    present: bool,
    capacity: u8,
    fake_capacity: Option<u8>,
    status: &'static str,
    health: &'static str,
    voltage_now_uv: i32,
    current_now_ua: i32,
    current_avg_ua: i32,
    charge_counter_uah: i32,
    charge_full_uah: i32,
    charge_full_design_uah: i32,
    temp_celsius_tenths: i32,
    cycle_count: i32,
    capacity_level: &'static str,
    time_to_full_now_seconds: Option<i64>,
    technology: &'static str,
}

impl Default for BatteryState {
    fn default() -> Self {
        Self {
            present: true,
            capacity: 0,
            fake_capacity: None,
            status: "Unknown",
            health: "Unknown",
            voltage_now_uv: 0,
            current_now_ua: 0,
            current_avg_ua: 0,
            charge_counter_uah: 0,
            charge_full_uah: 0,
            charge_full_design_uah: 0,
            temp_celsius_tenths: 0,
            cycle_count: 0,
            capacity_level: "Unknown",
            time_to_full_now_seconds: None,
            technology: "Li-ion",
        }
    }
}

impl BatteryState {
    fn capacity_level_for_percent(capacity: u8, status: &str) -> &'static str {
        match capacity {
            100.. => "Full",
            80..=99 => "High",
            40..=79 => "Normal",
            1..=39 => "Low",
            0 if status == "Charging" => "Low",
            0 => "Critical",
        }
    }

    fn effective_capacity(&self) -> u8 {
        self.fake_capacity.unwrap_or(self.capacity)
    }

    fn effective_capacity_level(&self) -> &'static str {
        match self.fake_capacity {
            Some(cap) => Self::capacity_level_for_percent(cap, self.status),
            None => self.capacity_level,
        }
    }

    /// Returns a normalized copy with `effective_capacity()` and `effective_capacity_level()`
    /// materialized so `PartialEq` reflects user-visible sysfs changes.
    fn effective(&self) -> Self {
        Self {
            capacity: self.effective_capacity(),
            capacity_level: self.effective_capacity_level(),
            fake_capacity: None,
            ..*self
        }
    }
}

/// In-memory state of a charger power supply device (`usb`, `ac`, or `wireless`).
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct ChargerState {
    online: bool,
    current_max_ua: i32,
    voltage_max_uv: i32,
}

impl Default for ChargerState {
    fn default() -> Self {
        Self { online: false, current_max_ua: 0, voltage_max_uv: DEFAULT_CHARGER_MAX_VOLTAGE_UV }
    }
}

/// In-memory representation of all power supply devices.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PowerSupplyState {
    battery: BatteryState,
    usb: ChargerState,
    usb_type: &'static str,
    ac: ChargerState,
    wireless: ChargerState,
}

impl Default for PowerSupplyState {
    fn default() -> Self {
        Self {
            battery: BatteryState::default(),
            usb: ChargerState::default(),
            usb_type: USB_TYPE_UNKNOWN,
            ac: ChargerState::default(),
            wireless: ChargerState::default(),
        }
    }
}

type SharedPowerSupplyState = Arc<LockDepRwLock<PowerSupplyState, PowerSupplyStateLock>>;

struct PowerSupplyFile<F> {
    state: SharedPowerSupplyState,
    read_fn: F,
}

impl<T, F> PowerSupplyFile<F>
where
    T: Display + 'static,
    F: Fn(&PowerSupplyState) -> T + Send + Sync + 'static,
{
    fn new_node(state: &SharedPowerSupplyState, read_fn: F) -> impl FsNodeOps {
        BytesFile::new_node(Self { state: Arc::clone(state), read_fn })
    }
}

impl<T, F> BytesFileOps for PowerSupplyFile<F>
where
    T: Display + 'static,
    F: Fn(&PowerSupplyState) -> T + Send + Sync + 'static,
{
    fn read(&self, _current_task: &CurrentTask) -> Result<Cow<'_, [u8]>, Errno> {
        let state = self.state.read();
        Ok(serialize_for_file((self.read_fn)(&state)).into())
    }
}

fn power_supply_file<T, F>(
    dir: &SimpleDirectoryMutator,
    name: &'static str,
    state: &SharedPowerSupplyState,
    read_fn: F,
) where
    T: Display + 'static,
    F: Fn(&PowerSupplyState) -> T + Send + Sync + 'static,
{
    dir.entry(name, PowerSupplyFile::new_node(state, read_fn), mode!(IFREG, 0o444));
}

struct BatteryCapacityFile {
    state: SharedPowerSupplyState,
    device: Device,
}

impl BatteryCapacityFile {
    fn new_node(state: &SharedPowerSupplyState, device: &Device) -> impl FsNodeOps {
        BytesFile::new_node(Self { state: Arc::clone(state), device: device.clone() })
    }
}

impl BytesFileOps for BatteryCapacityFile {
    fn read(&self, _current_task: &CurrentTask) -> Result<Cow<'_, [u8]>, Errno> {
        let state = self.state.read();
        Ok(serialize_for_file(state.battery.effective_capacity()).into())
    }

    fn write(&self, current_task: &CurrentTask, data: Vec<u8>) -> Result<(), Errno> {
        // TODO(https://fxbug.dev/574568519): Support passing a faked value on to BatteryManager.
        let s = std::str::from_utf8(&data).map_err(|_| errno!(EINVAL))?.trim();
        let val: i32 = s.parse().map_err(|_| errno!(EINVAL))?;
        let fake_capacity = if (0..=100).contains(&val) {
            Some(val as u8)
        } else if val < 0 {
            None
        } else {
            return error!(EINVAL);
        };
        let changed = {
            let mut state = self.state.write();
            let prev = state.battery.effective();
            state.battery.fake_capacity = fake_capacity;
            state.battery.effective() != prev
        };
        if changed {
            current_task
                .kernel()
                .device_registry
                .dispatch_uevent(UEventAction::Change, self.device.clone());
        }
        Ok(())
    }
}

/// Return a device directory builder that populates the common sysfs attributes (`type`, `online`,
/// optional `present`, `current_max`, and `voltage_max`) for a charger device (`usb`, `ac`, or
/// `wireless`).
fn build_charger_directory(
    charger_type: &'static [u8],
    state: &SharedPowerSupplyState,
    select: fn(&PowerSupplyState) -> &ChargerState,
    include_present: bool,
) -> impl Fn(&Device, &SimpleDirectoryMutator) + Send + Sync + 'static {
    let state = Arc::clone(state);
    move |device, dir| {
        build_device_directory(device, dir);
        dir.entry("type", BytesFile::new_node(charger_type.to_vec()), mode!(IFREG, 0o444));
        power_supply_file(dir, "online", &state, move |s| u8::from(select(s).online));
        if include_present {
            power_supply_file(dir, "present", &state, move |s| u8::from(select(s).online));
        }
        power_supply_file(dir, "current_max", &state, move |s| {
            let c = select(s);
            if c.online { c.current_max_ua } else { 0 }
        });
        power_supply_file(dir, "voltage_max", &state, move |s| {
            let c = select(s);
            if c.online { c.voltage_max_uv } else { 0 }
        });
    }
}

/// Initialize power supply devices under `/sys/class/power_supply/`.
pub fn battery_manager_device_init(kernel: &Kernel) {
    let registry = &kernel.device_registry;
    let power_supply_class = registry.objects.power_supply_class();

    let state = Arc::new(LockDepRwLock::new(PowerSupplyState::default()));

    // 1. /sys/class/power_supply/battery
    let state_clone = Arc::clone(&state);
    registry.add_numberless_device(
        "battery".into(),
        /* parent = */ None,
        power_supply_class.clone(),
        move |device, dir| {
            let state = &state_clone;
            build_device_directory(device, dir);
            dir.entry("type", BytesFile::new_node(b"Battery\n".to_vec()), mode!(IFREG, 0o444));
            power_supply_file(dir, "present", state, |s| u8::from(s.battery.present));
            dir.entry(
                "capacity",
                BatteryCapacityFile::new_node(state, device),
                mode!(IFREG, 0o644),
            );
            power_supply_file(dir, "status", state, |s| s.battery.status);
            power_supply_file(dir, "health", state, |s| s.battery.health);
            power_supply_file(dir, "voltage_now", state, |s| s.battery.voltage_now_uv);
            power_supply_file(dir, "current_now", state, |s| s.battery.current_now_ua);
            power_supply_file(dir, "current_avg", state, |s| s.battery.current_avg_ua);
            power_supply_file(dir, "charge_counter", state, |s| s.battery.charge_counter_uah);
            power_supply_file(dir, "charge_full", state, |s| s.battery.charge_full_uah);
            power_supply_file(dir, "charge_full_design", state, |s| {
                s.battery.charge_full_design_uah
            });
            power_supply_file(dir, "temp", state, |s| s.battery.temp_celsius_tenths);
            power_supply_file(dir, "cycle_count", state, |s| s.battery.cycle_count);
            power_supply_file(dir, "capacity_level", state, |s| {
                s.battery.effective_capacity_level()
            });
            // Linux's `google_battery.c` sets `val->intval = -EINVAL` (-22) with `err = 0`
            // when not charging, which Linux sysfs formats as `"-22\n"` (and Android
            // `BatteryMonitor` on Iris parses into `batteryChargeTimeToFullNowSeconds = -22`).
            power_supply_file(dir, "time_to_full_now", state, |s| {
                s.battery.time_to_full_now_seconds.unwrap_or(-(starnix_uapi::EINVAL as i64))
            });
            power_supply_file(dir, "technology", state, |s| s.battery.technology);
        },
    );

    // 2. /sys/class/power_supply/usb
    let state_clone = Arc::clone(&state);
    let build_usb =
        build_charger_directory(b"USB\n", &state, |s| &s.usb, /* include_present = */ true);
    registry.add_numberless_device(
        "usb".into(),
        /* parent = */ None,
        power_supply_class.clone(),
        move |device, dir| {
            build_usb(device, dir);
            power_supply_file(dir, "usb_type", &state_clone, |s| s.usb_type);
        },
    );

    // 3. /sys/class/power_supply/ac
    registry.add_numberless_device(
        "ac".into(),
        /* parent = */ None,
        power_supply_class.clone(),
        build_charger_directory(b"Mains\n", &state, |s| &s.ac, /* include_present = */ false),
    );

    // 4. /sys/class/power_supply/wireless
    registry.add_numberless_device(
        "wireless".into(),
        /* parent = */ None,
        power_supply_class,
        build_charger_directory(
            b"Wireless\n",
            &state,
            |s| &s.wireless,
            /* include_present = */ true,
        ),
    );

    // TODO(https://fxbug.dev/573313995): Watch BatteryManager and update the state.
}

#[cfg(test)]
mod tests {
    use super::*;
    use starnix_core::testing::{anon_test_file, spawn_kernel_and_run};
    use starnix_core::vfs::{FsNodeHandle, VecInputBuffer, VecOutputBuffer};
    use starnix_uapi::open_flags::OpenFlags;
    use std::sync::Mutex;

    fn read_sysfs_node(current_task: &CurrentTask, node: &FsNodeHandle) -> String {
        let ops = node.create_file_ops(current_task, OpenFlags::RDONLY).unwrap();
        let file = anon_test_file(current_task, ops, OpenFlags::RDONLY);
        let mut output = VecOutputBuffer::new(64);
        let n = file.read(current_task, &mut output).unwrap();
        String::from_utf8(Vec::from(output)[..n].to_vec()).unwrap()
    }

    #[::fuchsia::test]
    fn test_default_state() {
        let state = PowerSupplyState::default();
        assert!(state.battery.present);
        assert_eq!(state.battery.capacity, 0);
        assert_eq!(state.battery.fake_capacity, None);
        assert_eq!(state.battery.status, "Unknown");
        assert_eq!(state.battery.health, "Unknown");
        assert_eq!(state.battery.voltage_now_uv, 0);
        assert_eq!(state.battery.current_now_ua, 0);
        assert_eq!(state.battery.time_to_full_now_seconds, None);
        assert_eq!(state.battery.technology, "Li-ion");
        assert!(!state.usb.online);
        assert_eq!(state.usb_type, "[Unknown] SDP DCP CDP");
        assert_eq!(state.usb.voltage_max_uv, DEFAULT_CHARGER_MAX_VOLTAGE_UV);
        assert!(!state.ac.online);
        assert!(!state.wireless.online);
    }

    #[::fuchsia::test]
    async fn power_supply_registers_devices_and_attributes() {
        spawn_kernel_and_run(async |current_task| {
            let kernel = current_task.kernel();
            battery_manager_device_init(kernel);

            let root = &kernel.device_registry.objects.root;

            assert!(root.lookup("class/power_supply/battery".into()).is_some());
            assert!(root.lookup("class/power_supply/usb".into()).is_some());
            assert!(root.lookup("class/power_supply/ac".into()).is_some());
            assert!(root.lookup("class/power_supply/wireless".into()).is_some());

            let battery_capacity = root
                .lookup("devices/virtual/power_supply/battery/capacity".into())
                .expect("battery capacity node should exist");
            assert_eq!(battery_capacity.info().mode, mode!(IFREG, 0o644));
            assert!(
                root.lookup("devices/virtual/power_supply/battery/capacity_level".into()).is_some()
            );
            assert!(root.lookup("devices/virtual/power_supply/battery/status".into()).is_some());
            assert!(
                root.lookup("devices/virtual/power_supply/battery/voltage_now".into()).is_some()
            );
            assert!(root.lookup("devices/virtual/power_supply/battery/temp".into()).is_some());

            let ttf_node = root
                .lookup("devices/virtual/power_supply/battery/time_to_full_now".into())
                .expect("battery time_to_full_now node should exist");
            assert_eq!(read_sysfs_node(current_task, &ttf_node), "-22\n");

            assert!(root.lookup("devices/virtual/power_supply/usb/online".into()).is_some());
            assert!(root.lookup("devices/virtual/power_supply/usb/present".into()).is_some());
            assert!(root.lookup("devices/virtual/power_supply/usb/usb_type".into()).is_some());
            assert!(root.lookup("devices/virtual/power_supply/usb/current_max".into()).is_some());

            assert!(root.lookup("devices/virtual/power_supply/ac/online".into()).is_some());
            assert!(root.lookup("devices/virtual/power_supply/ac/present".into()).is_none());

            assert!(root.lookup("devices/virtual/power_supply/wireless/online".into()).is_some());
            assert!(root.lookup("devices/virtual/power_supply/wireless/present".into()).is_some());
        })
        .await;
    }

    #[::fuchsia::test]
    async fn battery_capacity_write_overrides_and_dispatches_uevent() {
        spawn_kernel_and_run(async |current_task| {
            let kernel = current_task.kernel();
            battery_manager_device_init(kernel);

            let registry = &kernel.device_registry;
            let root = &registry.objects.root;
            let battery_capacity = root
                .lookup("devices/virtual/power_supply/battery/capacity".into())
                .expect("battery capacity node should exist");
            let capacity_level_node = root
                .lookup("devices/virtual/power_supply/battery/capacity_level".into())
                .expect("battery capacity_level node should exist");

            let uevents = Arc::new(Mutex::new(Vec::new()));
            let uevents_clone = Arc::clone(&uevents);
            let listener_key =
                registry.register_listener(move |action, device: Device, _context| {
                    uevents_clone.lock().unwrap().push((action, device.name().to_string()));
                });

            let file_ops = battery_capacity.create_file_ops(current_task, OpenFlags::RDWR).unwrap();
            let file = anon_test_file(current_task, file_ops, OpenFlags::RDWR);
            assert_eq!(read_sysfs_node(current_task, &battery_capacity), "0\n");
            assert_eq!(read_sysfs_node(current_task, &capacity_level_node), "Unknown\n");

            let mut input = VecInputBuffer::new(b"77\n");
            assert_eq!(file.write(current_task, &mut input).unwrap(), 3);
            assert_eq!(read_sysfs_node(current_task, &battery_capacity), "77\n");
            assert_eq!(read_sysfs_node(current_task, &capacity_level_node), "Normal\n");
            assert_eq!(
                *uevents.lock().unwrap(),
                vec![(UEventAction::Change, "battery".to_string())]
            );

            // Writing the same capacity again should not emit a redundant uevent.
            let mut same_input = VecInputBuffer::new(b"77\n");
            assert_eq!(file.write(current_task, &mut same_input).unwrap(), 3);
            assert_eq!(uevents.lock().unwrap().len(), 1);

            let mut reset_input = VecInputBuffer::new(b"-1\n");
            assert_eq!(file.write(current_task, &mut reset_input).unwrap(), 3);
            assert_eq!(read_sysfs_node(current_task, &battery_capacity), "0\n");
            assert_eq!(read_sysfs_node(current_task, &capacity_level_node), "Unknown\n");
            assert_eq!(
                *uevents.lock().unwrap(),
                vec![
                    (UEventAction::Change, "battery".to_string()),
                    (UEventAction::Change, "battery".to_string()),
                ]
            );
            registry.unregister_listener(&listener_key);
        })
        .await;
    }
}
