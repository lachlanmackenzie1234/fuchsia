// Copyright 2019 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#pragma once

//
//
//

#include "path_builder.h"

//
//
//

struct spn_device;

//
//
//

spn_result
spn_path_builder_impl_create(struct spn_device         * const device,
                             struct spn_path_builder * * const path_builder);

//
//
//
