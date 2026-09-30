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
