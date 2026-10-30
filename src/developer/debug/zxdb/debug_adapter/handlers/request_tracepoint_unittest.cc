// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "src/developer/debug/zxdb/debug_adapter/handlers/request_tracepoint.h"

#include <cstdint>
#include <map>
#include <string>
#include <utility>
#include <vector>

#include <gtest/gtest.h>

#include "src/developer/debug/zxdb/client/session.h"
#include "src/developer/debug/zxdb/client/system.h"
#include "src/developer/debug/zxdb/client/tracepoint.h"
#include "src/developer/debug/zxdb/client/tracepoint_capture.h"
#include "src/developer/debug/zxdb/debug_adapter/context_test.h"

namespace zxdb {

namespace {

class RequestTracepointTest : public DebugAdapterContextTest {
 protected:
  dap::ResponseOrError<dap::ZxdbSetTracepointsResponse> SendSetTracepoints(
      std::vector<dap::ZxdbTracepointSpec> specs) {
    dap::ZxdbSetTracepointsRequest req;
    req.tracepoints = std::move(specs);
    auto response = client().send(req);
    context().OnStreamReadable();
    loop().RunUntilNoTasks();
    RunPendingClientCalls();
    return response.get();
  }
};

TEST_F(RequestTracepointTest, SetTracepointsRoundTrip) {
  InitializeDebugging();

  dap::ZxdbTracepointSpec spec;
  spec.function = "MyFunction";
  spec.max_matching_frames = 5;
  spec.max_hits = 10;
  spec.enabled = true;
  spec.expressions = dap::array<dap::string>{"x", "y"};
  spec.locals = true;
  spec.registers = true;

  auto got = SendSetTracepoints({spec});
  EXPECT_FALSE(got.error);
  ASSERT_EQ(got.response.tracepoints.size(), 1u);
  EXPECT_GT(got.response.tracepoints[0].id, 0);
  EXPECT_TRUE(got.response.tracepoints[0].max_matching_frames.has_value());
  EXPECT_EQ(*got.response.tracepoints[0].max_matching_frames, 5);
  EXPECT_EQ(got.response.tracepoints[0].max_hits, 10);
  EXPECT_TRUE(got.response.tracepoints[0].enabled);
  EXPECT_FALSE(got.response.tracepoints[0].auto_disabled);

  EXPECT_EQ(context().dap_tracepoints().size(), 1u);
  EXPECT_EQ(session().system().GetTracepoints().size(), 1u);
  auto it = context().dap_tracepoints().find(got.response.tracepoints[0].id);
  ASSERT_NE(it, context().dap_tracepoints().end());
  ASSERT_TRUE(it->second);
  EXPECT_TRUE(it->second->spec().max_matching_frames.has_value());
  EXPECT_EQ(*it->second->spec().max_matching_frames, 5u);
}

TEST_F(RequestTracepointTest, ReconciliationAndIdPreservation) {
  InitializeDebugging();

  dap::ZxdbTracepointSpec spec1;
  spec1.id = 10;
  spec1.function = "FuncA";
  spec1.max_hits = 5;
  spec1.enabled = true;

  auto got1 = SendSetTracepoints({spec1});
  EXPECT_FALSE(got1.error);
  ASSERT_EQ(got1.response.tracepoints.size(), 1u);
  EXPECT_EQ(got1.response.tracepoints[0].id, 10);

  auto it = context().dap_tracepoints().find(10);
  ASSERT_NE(it, context().dap_tracepoints().end());
  ASSERT_TRUE(it->second);
  it->second->set_hit_count(3);

  spec1.max_hits = 20;
  dap::ZxdbTracepointSpec spec2;
  spec2.function = "FuncB";

  auto got2 = SendSetTracepoints({spec1, spec2});
  EXPECT_FALSE(got2.error);
  ASSERT_EQ(got2.response.tracepoints.size(), 2u);
  EXPECT_EQ(got2.response.tracepoints[0].id, 10);
  EXPECT_EQ(got2.response.tracepoints[0].hit_count, 3);
  EXPECT_EQ(got2.response.tracepoints[0].max_hits, 20);

  int64_t new_id = got2.response.tracepoints[1].id;
  EXPECT_NE(new_id, 10);
  EXPECT_EQ(context().dap_tracepoints().size(), 2u);

  spec2.id = new_id;
  auto got3 = SendSetTracepoints({spec2});
  EXPECT_FALSE(got3.error);
  ASSERT_EQ(got3.response.tracepoints.size(), 1u);
  EXPECT_EQ(got3.response.tracepoints[0].id, new_id);
  EXPECT_EQ(context().dap_tracepoints().size(), 1u);
  EXPECT_EQ(context().dap_tracepoints().find(10), context().dap_tracepoints().end());
}

TEST_F(RequestTracepointTest, WatchpointResolution) {
  InitializeDebugging();

  dap::ZxdbTracepointSpec spec;
  dap::ZxdbTracepointWatchSpec watch;
  watch.data_id = "1234:0x2000:8";
  watch.access_type = "readWrite";
  spec.watch = watch;

  auto got = SendSetTracepoints({spec});
  EXPECT_FALSE(got.error);
  ASSERT_EQ(got.response.tracepoints.size(), 1u);

  ASSERT_EQ(context().dap_tracepoints().size(), 1u);
  Tracepoint* tp = context().dap_tracepoints().begin()->second.get();
  ASSERT_NE(tp, nullptr);
  ASSERT_TRUE(tp->spec().watch.has_value());
  EXPECT_EQ(tp->spec().watch->address, 0x2000u);
  EXPECT_EQ(tp->spec().watch->byte_size, 8u);
  EXPECT_EQ(tp->spec().watch->access_type, debug_ipc::BreakpointType::kReadWrite);
}

TEST_F(RequestTracepointTest, TracepointEventEmissionAndNonDapFiltering) {
  InitializeDebugging();

  dap::ZxdbTracepointSpec spec;
  spec.id = 42;
  spec.function = "MyFunction";
  auto set_got = SendSetTracepoints({spec});
  ASSERT_FALSE(set_got.error);
  auto dap_it = context().dap_tracepoints().find(42);
  ASSERT_NE(dap_it, context().dap_tracepoints().end());
  Tracepoint* dap_tp = dap_it->second.get();
  ASSERT_NE(dap_tp, nullptr);

  bool event_received = false;
  dap::ZxdbTracepointEvent received_event;
  client().registerHandler([&](const dap::ZxdbTracepointEvent& evt) {
    event_received = true;
    received_event = evt;
  });

  // Console-owned tracepoints absent from dap_tracepoints_ must not emit DAP events.
  Tracepoint* console_tp = session().system().CreateNewTracepoint();
  TracepointHitRecord console_hit;
  console_hit.tracepoint_id = console_tp->id();
  context().OnTracepointHit(console_tp, console_hit);
  context().OnTracepointHit(nullptr, console_hit);
  loop().RunUntilNoTasks();
  RunPendingClientCalls();
  EXPECT_FALSE(event_received);

  TracepointHitRecord hit;
  hit.tracepoint_id = dap_tp->id();
  hit.location = "src/foo.cc:123";
  hit.hit_count = 3;
  hit.max_hits = 10;
  hit.thread_koid = 9999;
  hit.dwell_ms = 1.25;
  hit.truncated = true;
  hit.capture.frame = TracepointFrameSummary{
      .name = "MyFunction", .file = "src/foo.cc", .line = 123, .ip = 0x12345, .sp = 0x54321};
  hit.capture.registers = std::map<std::string, std::string>{{"rax", "0x1"}};
  hit.capture.locals =
      std::map<std::string, TracepointValue>{{"count", {.value = "7", .type = "int"}}};

  context().OnTracepointHit(dap_tp, hit);
  loop().RunUntilNoTasks();
  RunPendingClientCalls();

  EXPECT_TRUE(event_received);
  EXPECT_EQ(received_event.tracepoint_id, 42);
  EXPECT_EQ(received_event.location, "src/foo.cc:123");
  EXPECT_EQ(received_event.hit_count, 3);
  EXPECT_EQ(received_event.max_hits, 10);
  EXPECT_EQ(received_event.thread_koid, 9999);
  EXPECT_DOUBLE_EQ(received_event.dwell_ms, 1.25);
  EXPECT_TRUE(received_event.truncated);
  ASSERT_TRUE(received_event.capture.frame.has_value());
  EXPECT_EQ(received_event.capture.frame->name, "MyFunction");
  ASSERT_TRUE(received_event.capture.registers.has_value());
  EXPECT_NE(received_event.capture.registers->find("rax"), received_event.capture.registers->end());
  ASSERT_TRUE(received_event.capture.locals.has_value());
  EXPECT_NE(received_event.capture.locals->find("count"), received_event.capture.locals->end());
}

TEST_F(RequestTracepointTest, TracepointBacktraceElisionAndErrorEventEmission) {
  InitializeDebugging();

  dap::ZxdbTracepointSpec spec;
  spec.id = 77;
  spec.function = "MyFunction";
  auto got_set = SendSetTracepoints({spec});
  ASSERT_FALSE(got_set.error);
  auto tp_it = context().dap_tracepoints().find(77);
  ASSERT_NE(tp_it, context().dap_tracepoints().end());
  Tracepoint* tp = tp_it->second.get();
  ASSERT_NE(tp, nullptr);

  bool event_received = false;
  dap::ZxdbTracepointEvent received_event;
  client().registerHandler([&](const dap::ZxdbTracepointEvent& evt) {
    event_received = true;
    received_event = evt;
  });

  TracepointHitRecord hit;
  hit.tracepoint_id = tp->id();
  hit.location = "MyFunction";
  hit.hit_count = 1;
  hit.max_hits = 15;
  hit.thread_koid = 8888;
  TracepointFrameRecord rec;
  rec.frame_index = 0;
  rec.ip = 0x2000;
  rec.sp = 0x3000;
  rec.name = "AsyncExecutor";
  rec.elided = true;
  rec.elide_description = "2 async executor frames";
  hit.capture.backtrace = std::vector<TracepointFrameRecord>{rec};
  hit.capture.evaluations["x + 1"] = TracepointEvaluation{.result = "42", .type = "int"};
  hit.capture.errors = {"Condition error: bad_ident"};
  hit.capture.matched = false;
  hit.auto_disabled = true;

  context().OnTracepointHit(tp, hit);
  loop().RunUntilNoTasks();
  RunPendingClientCalls();

  EXPECT_TRUE(event_received);
  EXPECT_EQ(received_event.tracepoint_id, 77);
  EXPECT_EQ(received_event.location, "MyFunction");
  EXPECT_EQ(received_event.hit_count, 1);
  EXPECT_EQ(received_event.max_hits, 15);
  EXPECT_EQ(received_event.thread_koid, 8888);
  EXPECT_TRUE(received_event.auto_disabled);
  EXPECT_FALSE(received_event.capture.matched);
  ASSERT_TRUE(received_event.capture.backtrace.has_value());
  ASSERT_EQ(received_event.capture.backtrace->size(), 1u);
  EXPECT_TRUE((*received_event.capture.backtrace)[0].elided);
  EXPECT_EQ((*received_event.capture.backtrace)[0].elide_description.value(),
            "2 async executor frames");
  ASSERT_TRUE(received_event.capture.evaluations.has_value());
  EXPECT_TRUE(received_event.capture.evaluations->count("x + 1"));
  ASSERT_EQ(received_event.capture.errors.size(), 1u);
  EXPECT_EQ(received_event.capture.errors[0], "Condition error: bad_ident");

  context().WillDestroyTracepoint(tp);
  EXPECT_EQ(context().dap_tracepoints().find(77), context().dap_tracepoints().end());
}

TEST_F(RequestTracepointTest, InvalidDataIdInputsReturnUnverifiedInfo) {
  InitializeDebugging();

  for (const char* bad_id : {"0:0x2000:8", "1234_0x2000_8", "1234:0xffffffffffffffff:16"}) {
    dap::ZxdbTracepointSpec spec;
    spec.watch = dap::ZxdbTracepointWatchSpec{.data_id = bad_id};
    auto got = SendSetTracepoints({spec});
    EXPECT_FALSE(got.error) << bad_id;
    ASSERT_EQ(got.response.tracepoints.size(), 1u);
    EXPECT_FALSE(got.response.tracepoints[0].verified) << bad_id;
    EXPECT_TRUE(got.response.tracepoints[0].message.has_value()) << bad_id;
  }
}

TEST_F(RequestTracepointTest, NonPositiveRequestIntegersReturnDapError) {
  InitializeDebugging();

  EXPECT_TRUE(SendSetTracepoints({{.id = 0, .function = "F"}}).error);
  EXPECT_TRUE(SendSetTracepoints({{.file = "foo.cc", .line = 0}}).error);
  EXPECT_TRUE(SendSetTracepoints({{.function = "F", .max_matching_frames = 0}}).error);
  EXPECT_TRUE(SendSetTracepoints({{.function = "F", .max_hits = 0}}).error);

  dap::ZxdbTracepointSpec zero_watch;
  zero_watch.watch = dap::ZxdbTracepointWatchSpec{.address = 0x1000, .byte_size = 0};
  EXPECT_TRUE(SendSetTracepoints({zero_watch}).error);
}

TEST_F(RequestTracepointTest, RawBacktraceImpliesBacktrace) {
  InitializeDebugging();

  dap::ZxdbTracepointSpec spec;
  spec.function = "MyFunction";
  spec.raw_backtrace = true;
  spec.backtrace = false;

  auto got = SendSetTracepoints({spec});
  EXPECT_FALSE(got.error);
  ASSERT_EQ(got.response.tracepoints.size(), 1u);

  ASSERT_EQ(context().dap_tracepoints().size(), 1u);
  Tracepoint* tp = context().dap_tracepoints().begin()->second.get();
  ASSERT_NE(tp, nullptr);
  EXPECT_TRUE(tp->spec().raw_backtrace);
  EXPECT_TRUE(tp->spec().backtrace);
}

TEST_F(RequestTracepointTest, UnrecognizedAccessType) {
  InitializeDebugging();

  dap::ZxdbTracepointSpec sideways_spec;
  sideways_spec.watch =
      dap::ZxdbTracepointWatchSpec{.data_id = "1234:0x2000:8", .access_type = "sideways"};
  auto got_sideways = SendSetTracepoints({sideways_spec});
  ASSERT_FALSE(got_sideways.error);
  EXPECT_FALSE(got_sideways.response.tracepoints[0].verified);
  EXPECT_EQ(got_sideways.response.tracepoints[0].message.value(), "Unknown accessType: sideways");
}

TEST_F(RequestTracepointTest, DeleteDapTracepointsSafeguard) {
  InitializeDebugging();

  dap::ZxdbTracepointSpec spec1{.id = 1, .function = "Func1"};
  dap::ZxdbTracepointSpec spec2{.id = 2, .function = "Func2"};
  auto got = SendSetTracepoints({spec1, spec2});
  ASSERT_FALSE(got.error);
  EXPECT_EQ(context().dap_tracepoints().size(), 2u);

  context().DeleteDapTracepoints();
  EXPECT_TRUE(context().dap_tracepoints().empty());
}

TEST_F(RequestTracepointTest, DuplicateAndAutoAssignedIdCollision) {
  InitializeDebugging();

  // Duplicate explicit IDs in the same request fail with a DAP error.
  dap::ZxdbTracepointSpec dup1{.id = 5, .function = "FuncA"};
  dap::ZxdbTracepointSpec dup2{.id = 5, .function = "FuncB"};
  EXPECT_TRUE(SendSetTracepoints({dup1, dup2}).error);

  // Auto-assigned ID followed by explicit id 1 in the same request avoids collision.
  dap::ZxdbTracepointSpec spec_auto{.function = "FuncAuto"};
  dap::ZxdbTracepointSpec spec_explicit{.id = 1, .function = "FuncExplicit"};
  auto got = SendSetTracepoints({spec_auto, spec_explicit});
  EXPECT_FALSE(got.error);
  ASSERT_EQ(got.response.tracepoints.size(), 2u);
  EXPECT_EQ(got.response.tracepoints[0].id, 2);
  EXPECT_EQ(got.response.tracepoints[1].id, 1);
  EXPECT_EQ(context().dap_tracepoints().size(), 2u);
}

TEST_F(RequestTracepointTest, InvalidSpecDeletesOldTracepoint) {
  InitializeDebugging();

  dap::ZxdbTracepointSpec spec1{.id = 42, .function = "ValidFunction"};
  auto got1 = SendSetTracepoints({spec1});
  ASSERT_FALSE(got1.error);
  EXPECT_EQ(session().system().GetTracepoints().size(), 1u);

  dap::ZxdbTracepointSpec invalid_spec{.id = 42, .file = "foo.cc"};
  auto got2 = SendSetTracepoints({invalid_spec});
  ASSERT_FALSE(got2.error);
  ASSERT_EQ(got2.response.tracepoints.size(), 1u);
  EXPECT_FALSE(got2.response.tracepoints[0].verified);
  EXPECT_TRUE(session().system().GetTracepoints().empty());
  EXPECT_TRUE(context().dap_tracepoints().empty());
}

TEST_F(RequestTracepointTest, SpecAddressAndLineValidationErrors) {
  InitializeDebugging();

  auto got_addr = SendSetTracepoints({{.address = -10}});
  ASSERT_FALSE(got_addr.error);
  ASSERT_EQ(got_addr.response.tracepoints.size(), 1u);
  EXPECT_FALSE(got_addr.response.tracepoints[0].verified);
  ASSERT_TRUE(got_addr.response.tracepoints[0].message.has_value());
  EXPECT_EQ(got_addr.response.tracepoints[0].message.value(),
            "Invalid negative tracepoint address.");

  auto got_line = SendSetTracepoints({{.file = "main.cc"}});
  ASSERT_FALSE(got_line.error);
  ASSERT_EQ(got_line.response.tracepoints.size(), 1u);
  EXPECT_FALSE(got_line.response.tracepoints[0].verified);
  ASSERT_TRUE(got_line.response.tracepoints[0].message.has_value());
  EXPECT_EQ(got_line.response.tracepoints[0].message.value(),
            "Both file and line must be specified together for a line tracepoint.");

  dap::ZxdbTracepointSpec neg_watch;
  neg_watch.watch = dap::ZxdbTracepointWatchSpec{.address = -1};
  auto got_watch = SendSetTracepoints({neg_watch});
  ASSERT_FALSE(got_watch.error);
  ASSERT_EQ(got_watch.response.tracepoints.size(), 1u);
  EXPECT_FALSE(got_watch.response.tracepoints[0].verified);
  ASSERT_TRUE(got_watch.response.tracepoints[0].message.has_value());
  EXPECT_EQ(got_watch.response.tracepoints[0].message.value(), "Invalid negative watch address.");
}

TEST_F(RequestTracepointTest, WatchByteSizeOverrideAndOverflow) {
  InitializeDebugging();

  dap::ZxdbTracepointSpec overflow_spec;
  overflow_spec.watch =
      dap::ZxdbTracepointWatchSpec{.data_id = "1234:0xfffffffffffffffe:1", .byte_size = 4};
  auto got_overflow = SendSetTracepoints({overflow_spec});
  ASSERT_FALSE(got_overflow.error);
  ASSERT_EQ(got_overflow.response.tracepoints.size(), 1u);
  EXPECT_FALSE(got_overflow.response.tracepoints[0].verified);
  ASSERT_TRUE(got_overflow.response.tracepoints[0].message.has_value());
  EXPECT_EQ(got_overflow.response.tracepoints[0].message.value(),
            "Watch address range overflows 64-bit address space.");

  dap::ZxdbTracepointSpec override_spec;
  override_spec.watch = dap::ZxdbTracepointWatchSpec{.data_id = "1234:0x2000:4", .byte_size = 8};
  auto got_override = SendSetTracepoints({override_spec});
  ASSERT_FALSE(got_override.error);
  ASSERT_EQ(got_override.response.tracepoints.size(), 1u);
  auto it = context().dap_tracepoints().find(got_override.response.tracepoints[0].id);
  ASSERT_NE(it, context().dap_tracepoints().end());
  ASSERT_TRUE(it->second);
  ASSERT_TRUE(it->second->spec().watch.has_value());
  EXPECT_EQ(it->second->spec().watch->byte_size, 8u);
}

}  // namespace

}  // namespace zxdb
