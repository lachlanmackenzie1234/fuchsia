// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

// Temporary dependencies on kernel Rust crates to build them while we are
// working on bringing up more functionality in Rust.  At the moment, this
// code is not actually used by the kernel.

use counters_rs as _;
use debug as _;
use ksync as _;
use platform_rs as _;
use vm as _;
