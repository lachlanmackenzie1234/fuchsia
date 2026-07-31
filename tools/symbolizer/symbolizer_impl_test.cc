// Copyright 2020 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "tools/symbolizer/symbolizer_impl.h"

#include <lib/fit/defer.h>

#include <fstream>
#include <sstream>
#include <string_view>

#include <gmock/gmock.h>
#include <gtest/gtest.h>
#include <rapidjson/document.h>
#include <rapidjson/istreamwrapper.h>

#include "src/lib/files/scoped_temp_dir.h"
#include "tools/symbolizer/log_parser.h"
#include "tools/symbolizer/symbolizer.h"

namespace symbolizer {

namespace {

using ::analytics::google_analytics_4::Value;
using ::testing::ContainerEq;

class SymbolizerImplTest : public ::testing::Test {
 public:
  SymbolizerImplTest() : symbolizer_(options_) {}

 protected:
  Symbolizer::StringOutputFn GetOutputFn() {
    return [this](std::string_view s) { ss_ << s << '\n'; };
  }

  std::stringstream ss_;
  CommandLineOptions options_;
  SymbolizerImpl symbolizer_;
};

TEST_F(SymbolizerImplTest, Reset) {
  symbolizer_.Reset(false, Symbolizer::ResetType::kUnknown);
  ASSERT_TRUE(ss_.str().empty());

  symbolizer_.Reset(false, Symbolizer::ResetType::kUnknown);
  ASSERT_TRUE(ss_.str().empty());
}

TEST_F(SymbolizerImplTest, MMap) {
  symbolizer_.Module(0, "some_module", "deadbeef");
  symbolizer_.MMap(0x1000, 0x2000, 0, "r", 0x0, GetOutputFn());
  ASSERT_EQ(ss_.str(), "[[[ELF module #0x0 \"some_module\" BuildID=deadbeef 0x1000]]]\n");

  ss_.str("");
  symbolizer_.MMap(0x3000, 0x1000, 0, "r", 0x2000, GetOutputFn());
  ASSERT_TRUE(ss_.str().empty()) << ss_.str();

  symbolizer_.MMap(0x3000, 0x1000, 0, "r", 0x1000, GetOutputFn());
  ASSERT_EQ(ss_.str(), "symbolizer: Inconsistent base address.\n");

  ss_.str("");
  symbolizer_.MMap(0x5000, 0x1000, 1, "r", 0x0, GetOutputFn());
  ASSERT_EQ(ss_.str(), "symbolizer: Invalid module id.\n");
}

TEST_F(SymbolizerImplTest, Backtrace) {
  symbolizer_.Module(0, "some_module", "deadbeef");
  symbolizer_.MMap(0x1000, 0x2000, 0, "r", 0x0, GetOutputFn());

  ss_.str("");
  symbolizer_.Backtrace(0, 0x1004, Symbolizer::AddressType::kProgramCounter, "", GetOutputFn());
  ASSERT_EQ(ss_.str(), "   #0    0x0000000000001004 in <some_module>+0x4\n");

  ss_.str("");
  symbolizer_.Backtrace(1, 0x5000, Symbolizer::AddressType::kUnknown, "", GetOutputFn());
  ASSERT_EQ(ss_.str(), "   #1    0x0000000000005000 is not covered by any module\n");
}

TEST(SymbolizerImpl, OmitModuleLines) {
  std::stringstream ss;
  CommandLineOptions options;
  options.omit_module_lines = true;
  SymbolizerImpl symbolizer(options);
  auto output = [&](std::string_view s) { ss << s << '\n'; };

  symbolizer.Module(0, "some_module", "deadbeef");
  symbolizer.MMap(0x1000, 0x2000, 0, "r", 0x0, output);
  ASSERT_EQ(ss.str(), "");
}

TEST(SymbolizerImpl, DumpFile) {
  // Creates a temp file first.
  files::ScopedTempDir temp_dir;
  std::string temp_file;
  ASSERT_TRUE(temp_dir.NewTempFile(&temp_file));

  {
    std::stringstream ss;
    CommandLineOptions options;
    options.dumpfile_output = temp_file;
    SymbolizerImpl symbolizer(options);
    auto output = [&](std::string_view s) { ss << s << '\n'; };

    symbolizer.Module(0, "some_module", "deadbeef");
    symbolizer.MMap(0x1000, 0x2000, 0, "r", 0x0, output);
    symbolizer.DumpFile("type", "name");

    // Triggers the destructor of symbolizer.
  }

  std::ifstream ifs(temp_file);
  rapidjson::IStreamWrapper isw(ifs);
  rapidjson::Document d;
  d.ParseStream(isw);

  ASSERT_TRUE(d.IsArray());
  ASSERT_EQ(d.Size(), 1U);

  ASSERT_TRUE(d[0].HasMember("modules"));
  ASSERT_TRUE(d[0]["modules"].IsArray());
  ASSERT_EQ(d[0]["modules"].Size(), 1U);
  ASSERT_TRUE(d[0]["modules"][0].IsObject());

  ASSERT_TRUE(d[0].HasMember("segments"));
  ASSERT_TRUE(d[0]["segments"].IsArray());
  ASSERT_EQ(d[0]["segments"].Size(), 1U);
  ASSERT_TRUE(d[0]["segments"][0].IsObject());

  ASSERT_TRUE(d[0].HasMember("type"));
  ASSERT_TRUE(d[0]["type"].IsString());
  ASSERT_EQ(std::string(d[0]["type"].GetString()), "type");

  ASSERT_TRUE(d[0].HasMember("name"));
  ASSERT_TRUE(d[0]["name"].IsString());
  ASSERT_EQ(std::string(d[0]["name"].GetString()), "name");
}

TEST(SymbolizerImpl, Analytics) {
  std::stringstream ss;
  CommandLineOptions options;
  std::string measurement_json;
  auto sender = [&measurement_json](const std::string& body) { measurement_json = body; };
  auto deferred_cleanup_analytics = fit::defer(Analytics::CleanUp);
  Analytics::InitTestingClient(std::move(sender));

  SymbolizerImpl symbolizer(options);
  auto output = [&](std::string_view s) { ss << s << '\n'; };

  symbolizer.Reset(false, Symbolizer::ResetType::kUnknown);
  symbolizer.Module(0, "some_module", "deadbeef");
  symbolizer.MMap(0x1000, 0x2000, 0, "r", 0x0, output);
  symbolizer.Backtrace(0, 0x1010, Symbolizer::AddressType::kUnknown, "", output);
  symbolizer.Backtrace(1, 0x7010, Symbolizer::AddressType::kUnknown, "", output);
  symbolizer.Reset(false, Symbolizer::ResetType::kUnknown);

  rapidjson::Document measurement;
  measurement.Parse(measurement_json);
  ASSERT_TRUE(measurement.HasMember("events"));
  auto& events = measurement["events"];
  ASSERT_TRUE(events.IsArray());
  ASSERT_GT(events.Size(), 0u);
  auto& event = events[0];
  ASSERT_TRUE(event.HasMember("params"));
  auto& params = event["params"];

  ASSERT_TRUE(params.HasMember("has_invalid_input"));
  EXPECT_EQ(params["has_invalid_input"].GetBool(), false);
  ASSERT_TRUE(params.HasMember("num_modules"));
  EXPECT_EQ(params["num_modules"].GetInt(), 1);
  ASSERT_TRUE(params.HasMember("num_modules_local"));
  EXPECT_EQ(params["num_modules_local"].GetInt(), 0);
  ASSERT_TRUE(params.HasMember("num_modules_cached"));
  EXPECT_EQ(params["num_modules_cached"].GetInt(), 0);
  ASSERT_TRUE(params.HasMember("num_modules_downloaded"));
  EXPECT_EQ(params["num_modules_downloaded"].GetInt(), 0);
  ASSERT_TRUE(params.HasMember("num_modules_download_fail"));
  EXPECT_EQ(params["num_modules_download_fail"].GetInt(), 0);
  ASSERT_TRUE(params.HasMember("num_frames"));
  EXPECT_EQ(params["num_frames"].GetInt(), 2);
  ASSERT_TRUE(params.HasMember("num_frames_symbolized"));
  EXPECT_EQ(params["num_frames_symbolized"].GetInt(), 0);
  ASSERT_TRUE(params.HasMember("num_frames_invalid"));
  EXPECT_EQ(params["num_frames_invalid"].GetInt(), 1);
  ASSERT_TRUE(params.HasMember("remote_symbol_enabled"));
  EXPECT_EQ(params["remote_symbol_enabled"].GetBool(), false);
  ASSERT_TRUE(params.HasMember("time_download_ms"));
  EXPECT_GE(params["time_total_ms"].GetInt(), 0);
}

TEST(SymbolizerImpl, MultipleMarkupOnSingleLine) {
  std::stringstream input;
  std::stringstream output;
  CommandLineOptions options;
  SymbolizerImpl symbolizer(options);
  LogParser parser(input, output, &symbolizer);

  input
      << "{{{module:0:some_module:elf:deadbeef}}}{{{mmap:0x1000:0x2000:load:0:r:0}}}{{{bt:0:0x1004:pc}}}\n";
  ASSERT_TRUE(parser.ProcessNextLine());
  EXPECT_EQ(output.str(),
            "[[[ELF module #0x0 \"some_module\" BuildID=deadbeef 0x1000]]]\n"
            "   #0    0x0000000000001004 in <some_module>+0x4\n");
}

TEST(SymbolizerImpl, MultipleMarkupWithSurroundingText) {
  std::stringstream input;
  std::stringstream output;
  CommandLineOptions options;
  SymbolizerImpl symbolizer(options);
  LogParser parser(input, output, &symbolizer);

  input
      << "prefix1 {{{module:0:some_module:elf:deadbeef}}} prefix2 {{{mmap:0x1000:0x2000:load:0:r:0}}} prefix3 {{{bt:0:0x1004:pc}}} suffix\n";
  ASSERT_TRUE(parser.ProcessNextLine());
  EXPECT_EQ(output.str(),
            "prefix1  prefix2 [[[ELF module #0x0 \"some_module\" BuildID=deadbeef 0x1000]]]\n"
            " prefix3    #0    0x0000000000001004 in <some_module>+0x4 suffix\n");
}

TEST(SymbolizerImpl, DroppedFinalTagWithTrailingSuffix) {
  std::stringstream input;
  std::stringstream output;
  CommandLineOptions options;
  SymbolizerImpl symbolizer(options);
  LogParser parser(input, output, &symbolizer);

  input << "{{{bt:0:0x1004:pc}}}{{{reset}}}tail\n";
  ASSERT_TRUE(parser.ProcessNextLine());
  EXPECT_EQ(output.str(), "   #0    0x0000000000001004 is not covered by any module\ntail\n");
}

TEST(SymbolizerImpl, MultipleMarkupBatchMode) {
  std::stringstream input;
  std::stringstream output;
  CommandLineOptions options;
  options.prettify_backtrace = true;
  SymbolizerImpl symbolizer(options);
  LogParser parser(input, output, &symbolizer);

  input << "{{{reset:begin}}}\n";
  input
      << "prefix {{{module:0:some_module:elf:deadbeef}}} {{{mmap:0x1000:0x2000:load:0:r:0}}} {{{bt:0:0x1004:pc}}} {{{invalid_tag}}} suffix\n";
  input << "{{{reset:end}}}\n";

  while (parser.ProcessNextLine()) {
  }

  EXPECT_EQ(output.str(),
            "prefix  [[[ELF module #0x0 \"some_module\" BuildID=deadbeef 0x1000]]]\n"
            "   #0 0x000000001004 some_module(deadbeef)+0x4\n"
            " {{{invalid_tag}}} suffix\n");
}

TEST(SymbolizerImpl, MalformedAndUnclosedTagsOnSingleLine) {
  std::stringstream input;
  std::stringstream output;
  CommandLineOptions options;
  SymbolizerImpl symbolizer(options);
  LogParser parser(input, output, &symbolizer);

  input
      << "{{{module:0:some_module:elf:deadbeef}}}{{{mmap:0x1000:0x2000:load:0:r:0}}}{{{bt:0:0x1004:pc}}}{{{invalid{{{reset}}}\n";
  ASSERT_TRUE(parser.ProcessNextLine());
  EXPECT_EQ(output.str(),
            "[[[ELF module #0x0 \"some_module\" BuildID=deadbeef 0x1000]]]\n"
            "   #0    0x0000000000001004 in <some_module>+0x4\n"
            "{{{invalid{{{reset}}}\n");
}

TEST(SymbolizerImpl, ZeroLengthPrefixesAndSuffixes) {
  std::stringstream input;
  std::stringstream output;
  CommandLineOptions options;
  SymbolizerImpl symbolizer(options);
  LogParser parser(input, output, &symbolizer);

  input
      << "{{{module:0:some_module:elf:deadbeef}}}{{{mmap:0x1000:0x2000:load:0:r:0}}}{{{bt:0:0x1004:pc}}}{{{bt:1:0x1008:pc}}}\n";
  ASSERT_TRUE(parser.ProcessNextLine());
  EXPECT_EQ(output.str(),
            "[[[ELF module #0x0 \"some_module\" BuildID=deadbeef 0x1000]]]\n"
            "   #0    0x0000000000001004 in <some_module>+0x4\n"
            "   #1    0x0000000000001008 in <some_module>+0x8\n");
}

TEST(SymbolizerImpl, TenPlusConsecutiveTagsOnSingleLine) {
  std::stringstream input;
  std::stringstream output;
  CommandLineOptions options;
  SymbolizerImpl symbolizer(options);
  LogParser parser(input, output, &symbolizer);

  input << "{{{module:0:some_module:elf:deadbeef}}}{{{mmap:0x1000:0x2000:load:0:r:0}}}";
  for (int i = 0; i < 12; ++i) {
    std::stringstream tag_ss;
    tag_ss << "{{{bt:" << std::dec << i << ":0x" << std::hex << (0x1004 + i * 4) << ":pc}}}";
    input << tag_ss.str();
  }
  input << "\n";

  ASSERT_TRUE(parser.ProcessNextLine());
  EXPECT_EQ(output.str(),
            "[[[ELF module #0x0 \"some_module\" BuildID=deadbeef 0x1000]]]\n"
            "   #0    0x0000000000001004 in <some_module>+0x4\n"
            "   #1    0x0000000000001008 in <some_module>+0x8\n"
            "   #2    0x000000000000100c in <some_module>+0xc\n"
            "   #3    0x0000000000001010 in <some_module>+0x10\n"
            "   #4    0x0000000000001014 in <some_module>+0x14\n"
            "   #5    0x0000000000001018 in <some_module>+0x18\n"
            "   #6    0x000000000000101c in <some_module>+0x1c\n"
            "   #7    0x0000000000001020 in <some_module>+0x20\n"
            "   #8    0x0000000000001024 in <some_module>+0x24\n"
            "   #9    0x0000000000001028 in <some_module>+0x28\n"
            "   #10   0x000000000000102c in <some_module>+0x2c\n"
            "   #11   0x0000000000001030 in <some_module>+0x30\n");
}

TEST(SymbolizerImpl, IntermingledValidBacktracesModuleResetAndText) {
  std::stringstream input;
  std::stringstream output;
  CommandLineOptions options;
  SymbolizerImpl symbolizer(options);
  LogParser parser(input, output, &symbolizer);

  input
      << "Header: {{{module:0:some_module:elf:deadbeef}}} setup {{{mmap:0x1000:0x2000:load:0:r:0}}} trace {{{bt:0:0x1004:pc}}} reset {{{reset}}} end {{{bt:0:0x1004:pc}}} tail\n";
  ASSERT_TRUE(parser.ProcessNextLine());
  EXPECT_EQ(output.str(),
            "Header:  setup [[[ELF module #0x0 \"some_module\" BuildID=deadbeef 0x1000]]]\n"
            " trace    #0    0x0000000000001004 in <some_module>+0x4\n"
            " reset  end    #0    0x0000000000001004 is not covered by any module tail\n");
}

}  // namespace

}  // namespace symbolizer
