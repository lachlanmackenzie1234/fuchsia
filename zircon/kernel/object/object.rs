// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#![no_std]

mod counter_dispatcher;
mod counter_dispatcher_ffi;
mod dispatcher;
mod dispatcher_ffi;
mod handle;
mod log_dispatcher;
mod log_dispatcher_ffi;
mod process_dispatcher;
mod process_dispatcher_ffi;
mod resource_ffi;
mod sampler_dispatcher;
mod sampler_dispatcher_ffi;

pub use counter_dispatcher::CounterDispatcher;
pub use dispatcher::{Dispatcher, DispatcherOps};
pub use handle::{HandleValue, KernelHandle};
pub use log_dispatcher::*;
pub use process_dispatcher::ProcessDispatcher;
pub use resource_ffi::{validate_ranged_resource, validate_resource_kind_base};
pub use sampler_dispatcher::SamplerDispatcher;
pub use sampler_dispatcher_ffi::*;
