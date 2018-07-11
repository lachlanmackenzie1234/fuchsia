// Copyright 2018 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include <errno.h>
#include <string.h>
#include <unistd.h>

#include "gtest/gtest.h"

TEST(FeaturesTest, NoPersistentStorage) {
  struct stat stat_;
  // /data is missing
  int retval = stat("/data", &stat_);
  ASSERT_EQ(retval, -1) << "Unexpectedly found /data";
  // While /svc is present
  retval = stat("/svc", &stat_);
  ASSERT_EQ(retval, 0) << "Can't find /svc: " << strerror(errno);
}
