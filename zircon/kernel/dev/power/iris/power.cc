// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include <debug.h>

#include <dev/power/iris/init.h>
#include <dev/psci.h>
#include <pdev/power.h>

namespace {

// Vendor-specific (bit 31) SYSTEM_RESET2 reset type to request a warm reset on Iris.
constexpr uint32_t kVendorSpecificWarmResetType = 0x80000000;

zx_status_t iris_reboot(power_reboot_flags flags) {
  switch (flags) {
    case power_reboot_flags::REBOOT_BOOTLOADER:
    case power_reboot_flags::REBOOT_RECOVERY:
    case power_reboot_flags::REBOOT_NORMAL:
      // The PMIC driver has already stashed the reboot reason which will survive cold reboot,
      // so we don't need to take any different action here.
      dprintf(INFO, "Iris reboot: performing cold reset\n");
      return psci_system_reset_cold();
    case power_reboot_flags::REBOOT_PANIC:
      dprintf(INFO, "Iris panic reboot: performing warm reset\n");
      // On Iris `kVendorSpecificWarmResetType` ignores the `cookie` parameter.
      return psci_system_reset2_raw(kVendorSpecificWarmResetType, 0);
    default:
      dprintf(INFO, "Iris reboot: unknown reboot flag %u, performing warm reset\n",
              static_cast<unsigned int>(flags));
      return psci_system_reset2_raw(kVendorSpecificWarmResetType, 0);
  }
}

const struct pdev_power_ops iris_power_ops = {
    .reboot = iris_reboot,
    .shutdown = psci_system_off,
    .cpu_off = psci_cpu_off,
    .cpu_on = psci_cpu_on,
    .get_cpu_state = psci_get_cpu_state,
};

}  // namespace

void iris_power_init_early() {
  dprintf(INFO, "POWER: registering iris power hooks\n");
  pdev_register_power(&iris_power_ops);
}
