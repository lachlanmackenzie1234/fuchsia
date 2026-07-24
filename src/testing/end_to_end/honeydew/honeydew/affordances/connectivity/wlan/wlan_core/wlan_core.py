# Copyright 2023 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.
"""Abstract base class for wlan affordance."""

import abc
from collections.abc import Sequence

import fidl_fuchsia_wlan_common as f_wlan_common
import fidl_fuchsia_wlan_device_service as f_wlan_device_service
import fidl_fuchsia_wlan_ieee80211 as f_wlan_ieee80211
import fidl_fuchsia_wlan_internal as f_wlan_internal
import fidl_fuchsia_wlan_sme as f_wlan_sme
from honeydew.affordances import affordance
from honeydew.affordances.connectivity.wlan.utils.types import (
    CountryCode,
    WlanInterfaces,
)


class AsyncWlanCore(abc.ABC):
    """Abstract base class for an async Wlan driver affordance."""

    @abc.abstractmethod
    async def connect(
        self,
        ssid: str,
        bss_desc: f_wlan_ieee80211.BssDescription,
        authentication: f_wlan_internal.Authentication,
    ) -> bool:
        """Trigger connection to a network.

        Args:
            ssid: The network to connect to.
            bss_desc: The basic service set for target network.
            authentication: Authentication to connect with.

        Returns:
            True on success otherwise false.

        Raises:
            HoneydewWlanError: Error from WLAN stack
            NetworkInterfaceNotFoundError: No client WLAN interface found.
        """

    @abc.abstractmethod
    async def create_iface(
        self,
        phy_id: int,
        role: f_wlan_common.WlanMacRole,
        sta_addr: str | None = None,
    ) -> int:
        """Create a new WLAN interface.

        Args:
            phy_id: The iface ID.
            role: The role of the new iface.
            sta_addr: MAC address for softAP iface.

        Returns:
            Iface id of newly created interface.

        Raises:
            HoneydewWlanError: Error from WLAN stack
            ValueError: Invalid MAC address
        """

    @abc.abstractmethod
    async def destroy_iface(self, iface_id: int) -> None:
        """Destroy WLAN interface by ID.

        Args:
            iface_id: The interface to destroy.

        Raises:
            HoneydewWlanError: Error from WLAN stack
        """

    @abc.abstractmethod
    async def disconnect(self) -> None:
        """Disconnect all client WLAN connections.

        Raises:
            HoneydewWlanError: Error from WLAN stack
        """

    @abc.abstractmethod
    async def get_iface_id_list(self) -> Sequence[int]:
        """Get list of wlan iface IDs on device.

        Returns:
            A list of wlan iface IDs that are present on the device.

        Raises:
            HoneydewWlanError: DeviceMonitor.ListIfaces error
        """

    @abc.abstractmethod
    async def get_country(self, phy_id: int) -> CountryCode:
        """Queries the currently configured country code from phy `phy_id`.

        Args:
            phy_id: A phy id that is present on the device.

        Returns:
            The currently configured country code from `phy_id`.

        Raises:
            HoneydewWlanError: DeviceMonitor.GetCountry error
        """

    @abc.abstractmethod
    async def set_country(self, phy_id: int, code: CountryCode) -> None:
        """Sets the country code for phy `phy_id`.

        Args:
            phy_id: A phy id that is present on the device.
            code: The country code to set.

        Raises:
            HoneydewWlanError: DeviceMonitor.SetCountry error
        """

    @abc.abstractmethod
    async def get_phy_id_list(self) -> Sequence[int]:
        """Get list of phy ids on device.

        Returns:
            A list of phy ids that is present on the device.

        Raises:
            HoneydewWlanError: DeviceMonitor.ListPhys error
        """

    @abc.abstractmethod
    async def query_interfaces(self) -> WlanInterfaces:
        """Retrieves a QueryIfaceResponse for every WLAN interface on the device.

        Returns:
            WlanInterfaces containing a QueryIfaceResponse for every WLAN interface
            on the device.

        Raises:
            HoneydewWlanError: DeviceMonitor.ListIfaces or DeviceMonitor.QueryIface error
        """

    @abc.abstractmethod
    async def query_iface(
        self, iface_id: int
    ) -> f_wlan_device_service.QueryIfaceResponse:
        """Retrieves interface info for given wlan iface id.

        Args:
            iface_id: The wlan interface id to get info from.

        Returns:
            QueryIfaceResponseWrapper from the SL4F server.

        Raises:
            HoneydewWlanError: DeviceMonitor.QueryIface error
        """

    @abc.abstractmethod
    async def scan_for_bss_info(
        self,
    ) -> dict[str, list[f_wlan_ieee80211.BssDescription]]:
        """Scans and returns BSS info.

        Returns:
            A dict mapping each seen SSID to a list of BSS Description IE
            blocks, one for each BSS observed in the network

        Raises:
            HoneydewWlanError: Error from WLAN stack
            NetworkInterfaceNotFoundError: No client WLAN interface found.
        """

    @abc.abstractmethod
    async def status(self) -> f_wlan_sme.ClientStatusResponse:
        """Request connection status

        Returns:
            fuchsia.wlan.sme/ClientStatusResponse FIDL union.

        Raises:
            HoneydewWlanError: Error from WLAN stack
            NetworkInterfaceNotFoundError: No client WLAN interface found.
            TypeError: If any of the return values are not of the expected type.
        """

    @abc.abstractmethod
    async def ensure_single_phy(self) -> int:
        """Wait for the first PHY device to appear, asserting no additional PHY devices are added.

        Returns:
            The phy_id of the single detected PHY.

        Raises:
            HoneydewWlanError: DeviceWatcher failed to report a phy or detected second phy.
        """


class WlanCore(affordance.Affordance):
    """Abstract base class for Wlan driver affordance."""

    # List all the public methods
    @abc.abstractmethod
    def connect(
        self,
        ssid: str,
        bss_desc: f_wlan_ieee80211.BssDescription,
        authentication: f_wlan_internal.Authentication,
    ) -> bool:
        """Trigger connection to a network.

        Args:
            ssid: The network to connect to.
            bss_desc: The basic service set for target network.
            authentication: Authentication to connect with.

        Returns:
            True on success otherwise false.

        Raises:
            HoneydewWlanError: Error from WLAN stack
            NetworkInterfaceNotFoundError: No client WLAN interface found.
        """

    @abc.abstractmethod
    def create_iface(
        self,
        phy_id: int,
        role: f_wlan_common.WlanMacRole,
        sta_addr: str | None = None,
    ) -> int:
        """Create a new WLAN interface.

        Args:
            phy_id: The iface ID.
            role: The role of the new iface.
            sta_addr: MAC address for softAP iface.

        Returns:
            Iface id of newly created interface.

        Raises:
            HoneydewWlanError: Error from WLAN stack
            ValueError: Invalid MAC address
        """

    @abc.abstractmethod
    def destroy_iface(self, iface_id: int) -> None:
        """Destroy WLAN interface by ID.

        Args:
            iface_id: The interface to destroy.

        Raises:
            HoneydewWlanError: Error from WLAN stack
        """

    @abc.abstractmethod
    def disconnect(self) -> None:
        """Disconnect all client WLAN connections.

        Raises:
            HoneydewWlanError: Error from WLAN stack
        """

    @abc.abstractmethod
    def get_iface_id_list(self) -> Sequence[int]:
        """Get list of wlan iface IDs on device.

        Returns:
            A list of wlan iface IDs that are present on the device.

        Raises:
            HoneydewWlanError: DeviceMonitor.ListIfaces error
        """

    @abc.abstractmethod
    def get_country(self, phy_id: int) -> CountryCode:
        """Queries the currently configured country code from phy `phy_id`.

        Args:
            phy_id: A phy id that is present on the device.

        Returns:
            The currently configured country code from `phy_id`.

        Raises:
            HoneydewWlanError: DeviceMonitor.GetCountry error
        """

    @abc.abstractmethod
    def set_country(self, phy_id: int, code: CountryCode) -> None:
        """Sets the country code for phy `phy_id`.

        Args:
            phy_id: A phy id that is present on the device.
            code: The country code to set.

        Raises:
            HoneydewWlanError: DeviceMonitor.SetCountry error
        """

    @abc.abstractmethod
    def get_phy_id_list(self) -> Sequence[int]:
        """Get list of phy ids on device.

        Returns:
            A list of phy ids that is present on the device.

        Raises:
            HoneydewWlanError: DeviceMonitor.ListPhys error
        """

    @abc.abstractmethod
    def query_interfaces(self) -> WlanInterfaces:
        """Retrieves a QueryIfaceResponse for every WLAN interface on the device.

        Returns:
            WlanInterfaces containing a QueryIfaceResponse for every WLAN interface
            on the device.

        Raises:
            HoneydewWlanError: DeviceMonitor.ListIfaces or DeviceMonitor.QueryIface error
        """

    @abc.abstractmethod
    def query_iface(
        self, iface_id: int
    ) -> f_wlan_device_service.QueryIfaceResponse:
        """Retrieves interface info for given wlan iface id.

        Args:
            iface_id: The wlan interface id to get info from.

        Returns:
            QueryIfaceResponseWrapper from the SL4F server.

        Raises:
            HoneydewWlanError: DeviceMonitor.QueryIface error
        """

    @abc.abstractmethod
    def scan_for_bss_info(
        self,
    ) -> dict[str, list[f_wlan_ieee80211.BssDescription]]:
        """Scans and returns BSS info.

        Returns:
            A dict mapping each seen SSID to a list of BSS Description IE
            blocks, one for each BSS observed in the network

        Raises:
            HoneydewWlanError: Error from WLAN stack
            NetworkInterfaceNotFoundError: No client WLAN interface found.
        """

    @abc.abstractmethod
    def status(self) -> f_wlan_sme.ClientStatusResponse:
        """Request connection status

        Returns:
            fuchsia.wlan.sme/ClientStatusResponse FIDL union.

        Raises:
            HoneydewWlanError: Error from WLAN stack
            NetworkInterfaceNotFoundError: No client WLAN interface found.
            TypeError: If any of the return values are not of the expected type.
        """
