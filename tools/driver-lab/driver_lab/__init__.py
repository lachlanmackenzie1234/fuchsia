# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

"""Driver-lab host tooling (phase 1).

Transport-neutral core: frozen data models, operator read-grant
resolution and persistence, and plan validation, canonicalization, and
digests. The target transport (fuchsia-controller over the
`fuchsia.driver.lab` wire contract) layers on top of these modules and is
deliberately kept out of them so this logic is testable on any host.
"""

from driver_lab.api import (
    DriverLab,
    DriverLabError,
    PollRecord,
    ReadRecord,
    RunResult,
    SequenceItemRecord,
    SequenceRecord,
    WriteRecord,
    connect,
)
from driver_lab.models import AccessClass, AccessRequest, WritePrecondition
from driver_lab.session import (
    AccessRequirements,
    Clock,
    Gpio,
    HardwareSession,
    I2c,
    Interrupt,
    MmioRegion,
    PollResult,
    ProtocolProxy,
    Reset,
    SequenceOutcome,
    Serial,
    SessionCapabilities,
    Spi,
    TranslationMetadata,
    UnsupportedCapabilityError,
    WriteResult,
)
from driver_lab.transport import (
    AllowRule,
    Denial,
    DirectDescription,
    DirectSession,
    DirectTransport,
    FidlCallOutcome,
    OperationDenied,
    PollOutcome,
    ProxyDescription,
    ProxySession,
    ProxyTransport,
    ResourceInfo,
    SequenceItem,
    SessionContext,
    SessionMode,
    SnapshotItem,
    TransportError,
    WriteOutcome,
)

__all__ = [
    "AccessClass",
    "AccessRequest",
    "AccessRequirements",
    "AllowRule",
    "Clock",
    "Denial",
    "DirectDescription",
    "DirectSession",
    "DirectTransport",
    "DriverLab",
    "DriverLabError",
    "FidlCallOutcome",
    "Gpio",
    "HardwareSession",
    "I2c",
    "Interrupt",
    "MmioRegion",
    "OperationDenied",
    "PollOutcome",
    "PollRecord",
    "PollResult",
    "ProtocolProxy",
    "ProxyDescription",
    "ProxySession",
    "ProxyTransport",
    "ReadRecord",
    "Reset",
    "ResourceInfo",
    "RunResult",
    "SequenceItem",
    "SequenceItemRecord",
    "SequenceOutcome",
    "SequenceRecord",
    "Serial",
    "SessionCapabilities",
    "SessionContext",
    "SessionMode",
    "SnapshotItem",
    "Spi",
    "TranslationMetadata",
    "TransportError",
    "UnsupportedCapabilityError",
    "WriteOutcome",
    "WritePrecondition",
    "WriteRecord",
    "WriteResult",
    "connect",
]
