// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "test-utils.h"

std::filesystem::path HelperPath(std::string_view name) {
  const char* root_dir = getenv("TEST_ROOT_DIR");
  if (!root_dir) {
    root_dir = "/pkg";
  }
  std::filesystem::path file(root_dir);
  file /= "bin";
  file /= name;
  EXPECT_TRUE(std::filesystem::exists(file))
      << '"' << file << "\" from TEST_ROOT_DIR=\"" << root_dir << '"';
  return file;
}
