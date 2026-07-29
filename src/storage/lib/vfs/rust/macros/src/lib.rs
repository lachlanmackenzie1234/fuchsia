// Copyright 2019 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

/// Converts a lower-case attribute name into the appropriate NodeAttributesQuery flag. This
/// is used by the attributes! macro.
#[proc_macro]
pub fn attribute_query(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    use std::str::FromStr;
    proc_macro::TokenStream::from_str(&format!(
        "fio::NodeAttributesQuery::{}",
        input.to_string().to_uppercase()
    ))
    .unwrap()
}
