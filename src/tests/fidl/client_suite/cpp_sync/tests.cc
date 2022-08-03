// Copyright 2022 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "src/tests/fidl/client_suite/cpp_sync/runner.h"

CLIENT_TEST(Setup) {}

CLIENT_TEST(GracefulFailureDuringCallAfterPeerClose) {
  auto result = target()->TwoWayNoPayload();
  ASSERT_EQ(ZX_ERR_PEER_CLOSED, result.error_value().status());
}

CLIENT_TEST(TwoWayNoPayload) {
  auto result = target()->TwoWayNoPayload();
  ASSERT_TRUE(result.is_ok()) << result.error_value();
}
