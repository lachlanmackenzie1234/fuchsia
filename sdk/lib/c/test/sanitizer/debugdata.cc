// Copyright 2019 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "debugdata.h"

#include <lib/async-loop/cpp/loop.h>
#include <lib/fdio/io.h>
#include <lib/fdio/spawn.h>
#include <lib/fit/defer.h>
#include <lib/ld/testing/mock-debugdata.h>
#include <lib/zx/channel.h>
#include <lib/zx/job.h>
#include <lib/zx/process.h>
#include <zircon/status.h>

#include <string>

#include "test-utils.h"

namespace {

using ::testing::AllOf;
using ::testing::Ne;

constexpr std::string_view kTestHelper = "debugdata-test-helper";

constexpr const char* kHelperPublishCommand = "publish_data";
constexpr const char* kHelperPublishFailCommand = "publish_data_fail";

void RunHelper(const char* mode, const size_t action_count, const fdio_spawn_action_t* fdio_actions,
               int expected_return_code) {
  zx::job test_job;
  ASSERT_OK(zx::job::create(*zx::job::default_job(), 0, &test_job));
  auto auto_call_kill_job = fit::defer([&test_job]() { test_job.kill(); });

  const auto path = HelperPath(kTestHelper);
  const char* args[] = {path.c_str(), mode, nullptr};

  zx::process process;
  char err_msg[FDIO_SPAWN_ERR_MSG_MAX_LENGTH];
  ASSERT_OK(fdio_spawn_etc(test_job.get(), FDIO_SPAWN_CLONE_ALL & ~FDIO_SPAWN_CLONE_NAMESPACE,
                           args[0], args, nullptr, action_count, fdio_actions,
                           process.reset_and_get_address(), err_msg));

  ASSERT_OK(process.wait_one(ZX_PROCESS_TERMINATED, zx::time::infinite(), nullptr));

  zx_info_process_t proc_info;
  ASSERT_OK(process.get_info(ZX_INFO_PROCESS, &proc_info, sizeof(proc_info), nullptr, nullptr));
  ASSERT_EQ(expected_return_code, proc_info.return_code);
}

void RunHelperWithSvc(const char* mode, fidl::ClientEnd<fuchsia_io::Directory> client_end,
                      int expected_return_code) {
  fdio_spawn_action_t fdio_actions[] = {
      fdio_spawn_action_t{
          .action = FDIO_SPAWN_ACTION_ADD_NS_ENTRY,
          .ns =
              {
                  .prefix = "/svc",
                  .handle = client_end.TakeChannel().release(),
              },
      },
  };
  RunHelper(mode, 1, fdio_actions, expected_return_code);
}

void RunHelperWithoutSvc(const char* mode, int expected_return_code) {
  RunHelper(mode, 0, nullptr, expected_return_code);
}

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

  ASSERT_NO_FATAL_FAILURE(RunHelperWithSvc(kHelperPublishCommand, std::move(svc_client_end), 0));

  ASSERT_OK(svc_dir.loop().RunUntilIdle());
}

TEST(DebugDataTests, PublishDataWithoutSvc) {
  ASSERT_NO_FATAL_FAILURE(RunHelperWithoutSvc(kHelperPublishFailCommand, 0));
}

TEST(DebugDataTests, PublishDataWithBadSvc) {
  zx::channel client_channel_end, server_channel_end;
  ASSERT_OK(zx::channel::create(0, &client_channel_end, &server_channel_end));
  fidl::ClientEnd<fuchsia_io::Directory> client_end{
      std::move(client_channel_end),
  };
  server_channel_end.reset();
  ASSERT_NO_FATAL_FAILURE(RunHelperWithSvc(kHelperPublishFailCommand, std::move(client_end), 0));
}

}  // anonymous namespace
