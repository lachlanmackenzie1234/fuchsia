// Copyright 2019 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "debugdata.h"

#include <lib/async-loop/cpp/loop.h>
#include <lib/fdio/io.h>
#include <lib/fit/defer.h>
#include <lib/ld/testing/mock-debugdata.h>
#include <lib/zx/channel.h>
#include <zircon/status.h>

#include <string>

#include "test-utils.h"

namespace {

using ::testing::_;
using ::testing::AllOf;
using ::testing::Ne;

constexpr std::string_view kTestHelper = "debugdata-test-helper";

constexpr const char* kHelperPublishCommand = "publish_data";
constexpr const char* kHelperPublishFailCommand = "publish_data_fail";

HelperResult RunHelperWithSvc(const char* mode, fidl::ClientEnd<fuchsia_io::Directory> client_end) {
  return RunHelper(kTestHelper, {mode},
                   {{.action = FDIO_SPAWN_ACTION_ADD_NS_ENTRY,
                     .ns = {
                         .prefix = "/svc",
                         .handle = client_end.TakeChannel().release(),
                     }}});
}

auto RunHelperWithoutSvc(const char* mode) { return RunHelper(kTestHelper, {mode}); }

TEST(DebugDataTests, PublishData) {
  auto mock = std::make_unique<::testing::StrictMock<ld::testing::MockDebugdata>>();
  EXPECT_CALL(*mock,
              Publish(kTestName,
                      AllOf(ld::testing::ObjNameMatches(kTestName),
                            ld::testing::VmoContentsMatch(std::string(
                                reinterpret_cast<const char*>(kTestData), sizeof(kTestData)))),
                      ld::testing::ObjKoidMatches(Ne(ZX_KOID_INVALID))));

  ld::testing::MockSvcDirectory svc_dir;
  ASSERT_NO_FATAL_FAILURE(svc_dir.Init());
  ASSERT_NO_FATAL_FAILURE(svc_dir.AddEntry<fuchsia_debugdata::Publisher>(std::move(mock)));

  fidl::ClientEnd<fuchsia_io::Directory> svc_client_end;
  ASSERT_NO_FATAL_FAILURE(svc_dir.Serve(svc_client_end));

  ASSERT_THAT(RunHelperWithSvc(kHelperPublishCommand, std::move(svc_client_end)),
              ExitsWith(0, "", ""));

  ASSERT_OK(svc_dir.loop().RunUntilIdle());
}

TEST(DebugDataTests, PublishDataWithoutSvc) {
  ASSERT_THAT(RunHelperWithoutSvc(kHelperPublishFailCommand), ExitsWith(0, "", ""));
}

TEST(DebugDataTests, PublishDataWithBadSvc) {
  zx::channel client_channel_end, server_channel_end;
  ASSERT_OK(zx::channel::create(0, &client_channel_end, &server_channel_end));
  fidl::ClientEnd<fuchsia_io::Directory> client_end{
      std::move(client_channel_end),
  };
  server_channel_end.reset();
  ASSERT_THAT(RunHelperWithSvc(kHelperPublishFailCommand, std::move(client_end)),
              ExitsWith(0, "", _));
}

}  // anonymous namespace
