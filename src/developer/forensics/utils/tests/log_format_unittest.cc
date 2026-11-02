// Copyright 2019 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "src/developer/forensics/utils/log_format.h"

#include <fuchsia/logger/cpp/fidl.h>
#include <lib/syslog/cpp/macros.h>
#include <lib/zx/time.h>

#include <string>
#include <vector>

#include <gtest/gtest.h>

namespace forensics {
namespace {

using ::testing::TestParamInfo;
using ::testing::TestWithParam;
using ::testing::ValuesIn;

constexpr zx::duration kLogMessageBaseTimestamp = zx::sec(15604);
constexpr uint64_t kLogMessageProcessId = 7559;
constexpr uint64_t kLogMessageThreadId = 7687;

fuchsia::logger::LogMessage BuildLogMessage(const int32_t severity, const std::string& text,
                                            const zx::duration timestamp_offset = zx::nsec(0),
                                            const std::vector<std::string>& tags = {}) {
  fuchsia::logger::LogMessage msg{};
  msg.time = zx::time_boot((kLogMessageBaseTimestamp + timestamp_offset).get());
  msg.pid = kLogMessageProcessId;
  msg.tid = kLogMessageThreadId;
  msg.tags = tags;
  msg.severity = severity;
  msg.msg = text;
  return msg;
}

TEST(LogFormatTest, Check_CorrectSeverity) {
  std::string log_message;

  log_message = Format(BuildLogMessage(FUCHSIA_LOG_INFO, "line 1"));
  EXPECT_EQ(log_message, "[15604.000][07559][07687][] INFO: line 1\n");

  log_message = Format(BuildLogMessage(FUCHSIA_LOG_WARNING, "line 2"));
  EXPECT_EQ(log_message, "[15604.000][07559][07687][] WARN: line 2\n");

  log_message = Format(BuildLogMessage(FUCHSIA_LOG_ERROR, "line 3"));
  EXPECT_EQ(log_message, "[15604.000][07559][07687][] ERROR: line 3\n");

  log_message = Format(BuildLogMessage(FUCHSIA_LOG_FATAL, "line 4"));
  EXPECT_EQ(log_message, "[15604.000][07559][07687][] FATAL: line 4\n");

  log_message = Format(BuildLogMessage(
      FUCHSIA_LOG_INFO + FUCHSIA_LOG_WARNING + FUCHSIA_LOG_ERROR + FUCHSIA_LOG_FATAL, "line 5"));
  EXPECT_EQ(log_message, "[15604.000][07559][07687][] INVALID: line 5\n");

  log_message = Format(BuildLogMessage(FUCHSIA_LOG_TRACE, "line 6"));
  EXPECT_EQ(log_message, "[15604.000][07559][07687][] TRACE: line 6\n");

  log_message = Format(BuildLogMessage(FUCHSIA_LOG_DEBUG, "line 7"));
  EXPECT_EQ(log_message, "[15604.000][07559][07687][] DEBUG: line 7\n");

  log_message = Format(BuildLogMessage(FUCHSIA_LOG_INFO - 1, "line 8"));
  EXPECT_EQ(log_message, "[15604.000][07559][07687][] VLOG(1): line 8\n");

  log_message = Format(BuildLogMessage(FUCHSIA_LOG_INFO - 12, "line 9"));
  EXPECT_EQ(log_message, "[15604.000][07559][07687][] VLOG(12): line 9\n");
}

TEST(LogFormatTest, Check_CorrectTime) {
  std::string log_message = Format(BuildLogMessage(FUCHSIA_LOG_WARNING, "line 1", zx::msec(1)));
  EXPECT_EQ(log_message, "[15604.001][07559][07687][] WARN: line 1\n");
}

TEST(LogFormatTest, Check_CorrectTags) {
  std::string log_message = Format(BuildLogMessage(FUCHSIA_LOG_INFO, "line 1", zx::msec(1),
                                                   /*tags=*/{"foo", "bar"}));
  EXPECT_EQ(log_message, "[15604.001][07559][07687][foo, bar] INFO: line 1\n");
}

TEST(LogFormatTest, Check_InsertsDroppedLogMessageSingular) {
  fuchsia::logger::LogMessage message = BuildLogMessage(FUCHSIA_LOG_INFO, "line 1", zx::msec(1),
                                                        /*tags=*/{"foo", "bar"});
  message.dropped_logs = 1;

  EXPECT_EQ(
      Format(message),
      "[15604.001][07559][07687][foo, bar] WARN: !!! DROPPED 1 LOG !!!\n[15604.001][07559][07687][foo, bar] INFO: line 1\n");
}

TEST(LogFormatTest, Check_InsertsDroppedLogMessageMultiple) {
  fuchsia::logger::LogMessage message = BuildLogMessage(FUCHSIA_LOG_INFO, "line 1", zx::msec(1),
                                                        /*tags=*/{"foo", "bar"});
  message.dropped_logs = 2;

  EXPECT_EQ(
      Format(message),
      "[15604.001][07559][07687][foo, bar] WARN: !!! DROPPED 2 LOGS !!!\n[15604.001][07559][07687][foo, bar] INFO: line 1\n");
}

struct MatchesLogMessageTestCase {
  std::string test_name;
  std::string input;
  bool expected;
};

using MatchesLogMessageTest = TestWithParam<MatchesLogMessageTestCase>;

TEST_P(MatchesLogMessageTest, MatchesLogMessage) {
  const MatchesLogMessageTestCase& test_case = GetParam();
  EXPECT_EQ(MatchesLogMessage(test_case.input), test_case.expected);
}

INSTANTIATE_TEST_SUITE_P(WithVariousInputs, MatchesLogMessageTest,
                         ValuesIn(std::vector<MatchesLogMessageTestCase>({
                             {
                                 .test_name = "FormattedMessage",
                                 .input = Format(BuildLogMessage(FUCHSIA_LOG_INFO, "line 1")),
                                 .expected = true,
                             },
                             {
                                 .test_name = "HeaderOnly",
                                 .input = "[15604.000][07559][07687]",
                                 .expected = true,
                             },
                             {
                                 .test_name = "EmptyString",
                                 .input = "",
                                 .expected = false,
                             },
                             {
                                 .test_name = "CannotDecodeMessage",
                                 .input = "!!! CANNOT DECODE!!!\n",
                                 .expected = false,
                             },
                             {
                                 .test_name = "MissingSeconds",
                                 .input = "[.000][07559][07687]",
                                 .expected = false,
                             },
                             {
                                 .test_name = "NonDigitInSeconds",
                                 .input = "[156a4.000][07559][07687]",
                                 .expected = false,
                             },
                             {
                                 .test_name = "MissingTimestampDot",
                                 .input = "[15604][07559][07687]",
                                 .expected = false,
                             },
                             {
                                 .test_name = "MissingMilliseconds",
                                 .input = "[15604.][07559][07687]",
                                 .expected = false,
                             },
                             {
                                 .test_name = "NonDigitInMilliseconds",
                                 .input = "[15604.0a0][07559][07687]",
                                 .expected = false,
                             },
                             {
                                 .test_name = "MissingProcessKoidOpenBracket",
                                 .input = "[15604.000]07559][07687]",
                                 .expected = false,
                             },
                             {
                                 .test_name = "MissingProcessKoid",
                                 .input = "[15604.000][][07687]",
                                 .expected = false,
                             },
                             {
                                 .test_name = "NonDigitInProcessKoid",
                                 .input = "[15604.000][075a9][07687]",
                                 .expected = false,
                             },
                             {
                                 .test_name = "MissingThreadKoidOpenBracket",
                                 .input = "[15604.000][07559]07687]",
                                 .expected = false,
                             },
                             {
                                 .test_name = "MissingThreadKoid",
                                 .input = "[15604.000][07559][]",
                                 .expected = false,
                             },
                             {
                                 .test_name = "NonDigitInThreadKoid",
                                 .input = "[15604.000][07559][076a7]",
                                 .expected = false,
                             },
                         })),
                         [](const TestParamInfo<MatchesLogMessageTestCase>& info) {
                           return info.param.test_name;
                         });

}  // namespace
}  // namespace forensics
