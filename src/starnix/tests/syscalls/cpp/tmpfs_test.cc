// Copyright 2024 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include <sched.h>
#include <stdlib.h>
#include <sys/fsuid.h>
#include <sys/mount.h>
#include <sys/prctl.h>
#include <sys/stat.h>
#include <unistd.h>

#include <gmock/gmock.h>
#include <gtest/gtest.h>
#include <linux/capability.h>

#include "fault_test.h"
#include "fault_test_suite.h"
#include "src/starnix/tests/syscalls/cpp/capabilities_helper.h"
#include "src/starnix/tests/syscalls/cpp/syscall_matchers.h"
#include "src/starnix/tests/syscalls/cpp/test_helper.h"

namespace {

int CreateTempFile() {
  char tmpl[] = "/tmp/tmpfile.XXXXXX";
  return mkstemp(tmpl);
}

INSTANTIATE_TEST_SUITE_P(TmpfsFaultTest, FaultFileTest, ::testing::Values(CreateTempFile));

TEST(TmpfsTest, DefaultStickyBit) {
  if (!test_helper::HasSysAdmin()) {
    GTEST_SKIP() << "need CAP_SYS_ADMIN to mount tmpfs";
  }

  test_helper::ScopedTempDir temp_dir;

  auto mount_result = test_helper::ScopedMount::Mount("none", temp_dir.path(), "tmpfs", 0, nullptr);
  ASSERT_THAT(mount_result, SyscallResultIsOk()) << "mount(tmpfs) failed";

  struct stat stat_buf;
  ASSERT_EQ(stat(temp_dir.path().c_str(), &stat_buf), 0);
  EXPECT_EQ(stat_buf.st_mode & 07777, 01777u) << "Sticky bit should be set on tmpfs root";
}

TEST(TmpfsTest, CasefoldDisabledByDefault) {
  if (!test_helper::HasSysAdmin()) {
    GTEST_SKIP() << "need CAP_SYS_ADMIN to mount tmpfs";
  }

  test_helper::ScopedTempDir temp_dir;

  ASSERT_THAT(test_helper::ScopedMount::Mount("none", temp_dir.path(), "tmpfs", 0, nullptr),
              SyscallResultIsOk());

  std::string dir_path = temp_dir.path() + "/dir";
  ASSERT_THAT(mkdir(dir_path.c_str(), 0777), SyscallSucceeds());

  EXPECT_THAT(test_helper::SetCasefold(dir_path, true), SyscallResultIsErrno(EOPNOTSUPP));
}

TEST(TmpfsTest, DefaultUidAndGidUseCallerFsCred) {
  if (!test_helper::HasSysAdmin() || !test_helper::HasCapability(CAP_SETUID) ||
      !test_helper::HasCapability(CAP_SETGID)) {
    GTEST_SKIP() << "need CAP_SYS_ADMIN, CAP_SETUID, and CAP_SETGID";
  }

  test_helper::ScopedTempDir temp_dir;

  test_helper::ForkHelper helper;
  helper.RunInForkedProcess([&] {
    ASSERT_THAT(unshare(CLONE_NEWNS), SyscallSucceeds());
    ASSERT_THAT(mount(nullptr, "/", nullptr, MS_PRIVATE | MS_REC, nullptr), SyscallSucceeds());

    constexpr uid_t kTestUid = 1082;
    constexpr gid_t kTestGid = 1082;
    ASSERT_THAT(prctl(PR_SET_KEEPCAPS, 1), SyscallSucceeds());
    ASSERT_THAT(setresgid(kTestGid, kTestGid, kTestGid), SyscallSucceeds());
    ASSERT_THAT(setresuid(kTestUid, kTestUid, kTestUid), SyscallSucceeds());

    test_helper::SetCapabilityEffective(CAP_SYS_ADMIN);
    test_helper::UnsetCapabilityEffective(CAP_DAC_OVERRIDE);

    ASSERT_THAT(mount("tmpfs", temp_dir.path().c_str(), "tmpfs", 0, "mode=0755"),
                SyscallSucceeds());

    struct stat stat_buf;
    ASSERT_THAT(stat(temp_dir.path().c_str(), &stat_buf), SyscallSucceeds());
    EXPECT_EQ(stat_buf.st_uid, kTestUid);
    EXPECT_EQ(stat_buf.st_gid, kTestGid);
    EXPECT_EQ(stat_buf.st_mode & 07777, 0755u);

    std::string subdir_path = temp_dir.path() + "/subdir";
    EXPECT_THAT(mkdir(subdir_path.c_str(), 0755), SyscallSucceeds());
  });
  ASSERT_TRUE(helper.WaitForChildren());
}

}  // namespace
