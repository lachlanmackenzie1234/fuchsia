// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

/// Try to convert from T to U and log any error.
pub fn convert_log_err<T, U>(val: T) -> Result<U, <U as TryFrom<T>>::Error>
where
    U: TryFrom<T>,
    <U as TryFrom<T>>::Error: std::error::Error,
{
    val.try_into().inspect_err(|e| log::error!("Conversion error: {e}"))
}
