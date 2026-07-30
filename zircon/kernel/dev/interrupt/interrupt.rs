// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptTriggerMode {
    Edge = 0,
    Level = 1,
}

impl InterruptTriggerMode {
    pub fn string(self) -> &'static str {
        match self {
            InterruptTriggerMode::Edge => "edge",
            InterruptTriggerMode::Level => "level",
        }
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptPolarity {
    High = 0,
    Low = 1,
}

impl InterruptPolarity {
    pub fn string(self) -> &'static str {
        match self {
            InterruptPolarity::High => "high",
            InterruptPolarity::Low => "low",
        }
    }
}
