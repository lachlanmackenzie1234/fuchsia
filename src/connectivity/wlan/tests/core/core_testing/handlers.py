# Copyright 2024 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.
"""
Protocol handlers for antlion tests of WLAN core.
"""

import logging

logger = logging.getLogger(__name__)


import asyncio
from dataclasses import dataclass
from typing import Any

import fidl_fuchsia_wlan_sme as fidl_sme
from fuchsia_controller_py import Channel


@dataclass
class ConnectTransactionContext:
    txn_queue: asyncio.Queue[
        fidl_sme.ConnectTransactionOnConnectResultRequest
        | fidl_sme.ConnectTransactionOnDisconnectRequest
        | fidl_sme.ConnectTransactionOnRoamResultRequest
        | fidl_sme.ConnectTransactionOnSignalReportRequest
        | fidl_sme.ConnectTransactionOnChannelSwitchedRequest
    ]
    server: Channel


class ConnectTransactionEventHandler(fidl_sme.ConnectTransactionEventHandler):
    def __init__(
        self,
        proxy: Channel,
        server: Channel,
        verbose: bool = True,
    ) -> None:
        self.proxy = proxy
        self.server = server
        # Defer initialization of parent class to __aenter__
        self.verbose = verbose

    def on_connect_result(
        self,
        request: fidl_sme.ConnectTransactionOnConnectResultRequest,
    ) -> None:
        if self.verbose:
            logger.info("Connect result: %s", request)
        self.txn_queue.put_nowait(request)

    def on_disconnect(
        self,
        request: fidl_sme.ConnectTransactionOnDisconnectRequest,
    ) -> None:
        if self.verbose:
            logger.info("Disconnect: %s", request)
        self.txn_queue.put_nowait(request)

    def on_roam_result(
        self,
        request: fidl_sme.ConnectTransactionOnRoamResultRequest,
    ) -> None:
        if self.verbose:
            logger.info("Roam result: %s", request)
        self.txn_queue.put_nowait(request)

    def on_signal_report(
        self,
        request: fidl_sme.ConnectTransactionOnSignalReportRequest,
    ) -> None:
        if self.verbose:
            logger.info("Signal report: %s", request)
        self.txn_queue.put_nowait(request)

    def on_channel_switched(
        self,
        request: fidl_sme.ConnectTransactionOnChannelSwitchedRequest,
    ) -> None:
        if self.verbose:
            logger.info("Channel switched: %s", request)
        self.txn_queue.put_nowait(request)

    async def __aenter__(self) -> ConnectTransactionContext:
        super().__init__(
            client=fidl_sme.ConnectTransactionClient(self.proxy.take())
        )
        self.txn_queue: asyncio.Queue[
            fidl_sme.ConnectTransactionOnConnectResultRequest
            | fidl_sme.ConnectTransactionOnDisconnectRequest
            | fidl_sme.ConnectTransactionOnRoamResultRequest
            | fidl_sme.ConnectTransactionOnSignalReportRequest
            | fidl_sme.ConnectTransactionOnChannelSwitchedRequest
        ] = asyncio.Queue()
        self.server_task = asyncio.create_task(self.serve())
        return ConnectTransactionContext(
            txn_queue=self.txn_queue,
            server=self.server,
        )

    async def __aexit__(self, *args: Any, **kwargs: Any) -> None:
        if self.server_task:
            self.server_task.cancel()
