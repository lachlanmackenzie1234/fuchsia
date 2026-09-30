# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

import argparse
import asyncio
import os
import sys
import unittest
from typing import Any, Dict

from dap_test_framework import (
    DapTestCase,
    get_dap_source_path,
)
from pydap.dap_types import DataBreakpoint, Source, SourceBreakpoint
from pydap.models import (
    ContinueArguments,
    DataBreakpointInfoArguments,
    InitializeArguments,
    LaunchArguments,
    ScopesArguments,
    SetBreakpointsArguments,
    SetDataBreakpointsArguments,
    VariablesArguments,
)
from zxdb_dap import ZxdbStackTraceArguments


class TestDapSmoke(DapTestCase):
    async def test_setup(self) -> None:
        # This test verifies that the setup, such as connecting to the DAP server, succeeds both locally and in the CQ
        pass


# Any tests that send initialize will automatically send disconnect after teardown
class TestDapInit(DapTestCase):
    auto_initialize = False

    async def test_initialize(self) -> None:
        await self.initialize(InitializeArguments(adapterID="zxdb"))

    async def test_initialize_partial(self) -> None:
        init_fut = self.initialize(InitializeArguments(adapterID="zxdb"))
        self.split_request(init_fut.request_seq, delay=0.1)
        await init_fut


class TestDapDisconnect(DapTestCase):
    async def test_disconnect_on_close(self) -> None:
        # Create a future to explicitly synchronize when the callback runs
        callback_run_future = asyncio.get_running_loop().create_future()

        async def close_socket_on_disconnect(
            writer: asyncio.StreamWriter, value: Dict[str, Any]
        ) -> None:
            try:
                writer.close()
                await writer.wait_closed()
            except Exception:
                pass
            finally:
                if not callback_run_future.done():
                    callback_run_future.set_result(True)

        # Trigger disconnect in background
        disconnect_fut = self.disconnect()

        # Register the callback for the disconnect request sequence
        self.set_sent_callback(
            disconnect_fut.request_seq, close_socket_on_disconnect
        )

        # Wait explicitly for the socket closure callback to execute and complete
        await asyncio.wait_for(callback_run_future, timeout=5.0)

        # CRITICAL: Dispose of the disconnect response since we closed the socket
        # immediately and do not expect a response, avoiding unretrieved exception warnings.
        self.dispose_response(disconnect_fut)

        # 3. Wait for the server process to exit voluntarily.
        try:
            await self.framework.wait_for_shutdown(timeout=10.0)
        except asyncio.TimeoutError:
            self.fail(
                "DAP server failed to exit after socket close with pending disconnect (hung/leaked!)"
            )


class TestDapBreakpoint(DapTestCase):
    # currently we don't use optimize=none because this test only test if we can hit the breakpoint.
    # if we need to check the file line of the breakpoint, then we should set optimize=none.
    async def test_breakpoint(self) -> None:
        crasher_path = get_dap_source_path(
            "src/developer/forensics/crasher/cpp/crasher.c"
        )
        bp_resp = await self.set_breakpoints(
            SetBreakpointsArguments(
                source=Source(path=crasher_path),
                breakpoints=[SourceBreakpoint(line=25)],
            )
        )
        self.assertTrue(bp_resp["success"])
        self.assertEqual(len(bp_resp["body"]["breakpoints"]), 1)
        bp_id = bp_resp["body"]["breakpoints"][0]["id"]

        self.launch(
            LaunchArguments(
                process="fuchsia-pkg://fuchsia.com/crasher#meta/cpp_crasher.cm"
            )
        )
        await self.on_event("stopped", 30.0).expect(
            {
                "body": {
                    "reason": "breakpoint",
                    "hitBreakpointIds": [bp_id],
                }
            }
        )

        clear_resp = await self.set_breakpoints(
            SetBreakpointsArguments(
                source=Source(path=crasher_path),
                breakpoints=[],
            )
        )
        self.assertTrue(clear_resp["success"])
        self.assertEqual(len(clear_resp["body"]["breakpoints"]), 0)


class TestDapBreakpointLine(DapTestCase):
    require_build_type = ["optimize=none"]

    # It's possible to have different file lines to be mapped into a same address when compiling.
    # Breakpoint is implemented based on replacing address of <file line> with a breakpoint trap.
    # when we hit the address, it is impossible to tell whether we are running fileLine1 or fileLine2.
    # To test against the line number, we set optimize=none.
    async def test_breakpoint_line(self) -> None:
        pretty_types_path = get_dap_source_path(
            "src/developer/debug/e2e_tests/inferiors/pretty_types.cc"
        )
        line_number = 38
        bp_resp = await self.set_breakpoints(
            SetBreakpointsArguments(
                source=Source(path=pretty_types_path),
                breakpoints=[SourceBreakpoint(line=line_number)],
            )
        )
        self.assertTrue(bp_resp["success"])
        self.assertEqual(len(bp_resp["body"]["breakpoints"]), 1)
        bp_id = bp_resp["body"]["breakpoints"][0]["id"]

        self.launch(
            LaunchArguments(
                process="fuchsia-pkg://fuchsia.com/zxdb_e2e_inferiors#meta/pretty_types.cm"
            )
        )
        stopped_event = await self.on_event("stopped", timeout=120.0)
        self.assertEqual(stopped_event["body"]["reason"], "breakpoint")
        self.assertIn(bp_id, stopped_event["body"]["hitBreakpointIds"])

        thread_id = stopped_event["body"]["threadId"]
        stack_resp = await self.zxdb_stack_trace(
            ZxdbStackTraceArguments(thread_id=thread_id, remote_unwind=True)
        )
        frames = stack_resp["body"]["stackFrames"]
        self.assertTrue(len(frames) > 0)

        self.assertEqual(frames[0]["line"], line_number)


class TestDapPrettyTypes(DapTestCase):
    require_build_type = ["optimize=none"]

    async def test_pretty_types(self) -> None:
        pretty_types_path = get_dap_source_path(
            "src/developer/debug/e2e_tests/inferiors/pretty_types.cc"
        )
        line_number = 38
        bp_resp = await self.set_breakpoints(
            SetBreakpointsArguments(
                source=Source(path=pretty_types_path),
                breakpoints=[SourceBreakpoint(line=line_number)],
            )
        )
        self.assertTrue(bp_resp["success"])
        self.assertEqual(len(bp_resp["body"]["breakpoints"]), 1)
        bp_id = bp_resp["body"]["breakpoints"][0]["id"]

        self.launch(
            LaunchArguments(
                process="fuchsia-pkg://fuchsia.com/zxdb_e2e_inferiors#meta/pretty_types.cm"
            )
        )
        stopped_event = await self.on_event("stopped", timeout=120.0)
        self.assertEqual(stopped_event["body"]["reason"], "breakpoint")
        self.assertIn(bp_id, stopped_event["body"]["hitBreakpointIds"])

        thread_id = stopped_event["body"]["threadId"]
        stack_resp = await self.zxdb_stack_trace(
            ZxdbStackTraceArguments(thread_id=thread_id, remote_unwind=True)
        )
        frames = stack_resp["body"]["stackFrames"]
        self.assertTrue(len(frames) > 0)
        frame_id = frames[0]["id"]

        scopes_resp = await self.scopes(ScopesArguments(frame_id=frame_id))
        self.assertTrue(scopes_resp["success"])
        locals_scope = next(
            s for s in scopes_resp["body"]["scopes"] if s["name"] == "Locals"
        )
        locals_ref = locals_scope["variablesReference"]

        vars_resp = await self.variables(
            VariablesArguments(variables_reference=locals_ref)
        )
        self.assertTrue(vars_resp["success"])
        vars_by_name = {v["name"]: v for v in vars_resp["body"]["variables"]}

        self.assertIn("vals", vars_by_name)
        self.assertIn("std::__2::vector", vars_by_name["vals"]["type"])
        self.assertIn("it", vars_by_name)
        self.assertEqual(vars_by_name["it"]["value"], "iterator")
        self.assertIn("sv", vars_by_name)
        if vars_by_name["sv"]["value"] != "<optimized out>":
            self.assertEqual(vars_by_name["sv"]["value"], '"abc"')
        self.assertIn("span", vars_by_name)
        self.assertIn("std::__2::span", vars_by_name["span"]["type"])

        # In DAP, compound collections (like std::vector) return an empty top-level
        # value string, while their elements are populated as child variables under
        # variablesReference.
        vals_ref = vars_by_name["vals"]["variablesReference"]
        self.assertGreater(vals_ref, 0)
        vals_children_resp = await self.variables(
            VariablesArguments(variables_reference=vals_ref)
        )
        self.assertTrue(vals_children_resp["success"])
        vals_children = {
            v["name"]: v["value"]
            for v in vals_children_resp["body"]["variables"]
        }
        self.assertEqual(vals_children["[0]"], "3")
        self.assertEqual(vals_children["[1]"], "4")
        self.assertEqual(vals_children["[2]"], "5")
        self.assertEqual(vals_children["[3]"], "6")

        # Expanding an iterator yields a dereferenced child node prefixed with '*'.
        it_ref = vars_by_name["it"]["variablesReference"]
        self.assertGreater(it_ref, 0)
        it_children_resp = await self.variables(
            VariablesArguments(variables_reference=it_ref)
        )
        self.assertTrue(it_children_resp["success"])
        it_children = {
            v["name"]: v["value"] for v in it_children_resp["body"]["variables"]
        }
        self.assertEqual(it_children["*it"], "3")


class TestDapPrettyTypesRust(DapTestCase):
    require_build_type = ["optimize=none"]

    async def test_pretty_types_rust(self) -> None:
        pretty_types_path = get_dap_source_path(
            "src/developer/debug/e2e_tests/inferiors/pretty_types.rs"
        )
        line_number = 51
        bp_resp = await self.set_breakpoints(
            SetBreakpointsArguments(
                source=Source(path=pretty_types_path),
                breakpoints=[SourceBreakpoint(line=line_number)],
            )
        )
        self.assertTrue(bp_resp["success"])
        self.assertEqual(len(bp_resp["body"]["breakpoints"]), 1)
        bp_id = bp_resp["body"]["breakpoints"][0]["id"]

        self.launch(
            LaunchArguments(
                process="fuchsia-pkg://fuchsia.com/zxdb_e2e_inferiors#meta/pretty_types_rust.cm"
            )
        )
        stopped_event = await self.on_event("stopped", timeout=120.0)
        self.assertEqual(stopped_event["body"]["reason"], "breakpoint")
        self.assertIn(bp_id, stopped_event["body"]["hitBreakpointIds"])

        thread_id = stopped_event["body"]["threadId"]
        stack_resp = await self.zxdb_stack_trace(
            ZxdbStackTraceArguments(thread_id=thread_id, remote_unwind=True)
        )
        frames = stack_resp["body"]["stackFrames"]
        self.assertTrue(len(frames) > 0)
        # Breakpoints on lines with function calls (e.g. heap.pop()) can stop in an
        # inlined standard library helper frame. Select the main frame for locals.
        main_frame = next(
            (f for f in frames if "main" in f.get("name", "")), frames[0]
        )
        frame_id = main_frame["id"]

        scopes_resp = await self.scopes(ScopesArguments(frame_id=frame_id))
        self.assertTrue(scopes_resp["success"])
        locals_scope = next(
            s for s in scopes_resp["body"]["scopes"] if s["name"] == "Locals"
        )
        locals_ref = locals_scope["variablesReference"]

        vars_resp = await self.variables(
            VariablesArguments(variables_reference=locals_ref)
        )
        self.assertTrue(vars_resp["success"])
        vars_by_name = {v["name"]: v for v in vars_resp["body"]["variables"]}

        self.assertIn("s", vars_by_name)
        self.assertEqual(vars_by_name["s"]["value"], '"hello"')
        self.assertIn("os_str", vars_by_name)
        self.assertEqual(vars_by_name["os_str"]["value"], '"osstr"')
        self.assertIn("heap", vars_by_name)
        self.assertIn("BinaryHeap", vars_by_name["heap"]["type"])
        self.assertIn("v", vars_by_name)
        self.assertIn("NestedVecs", vars_by_name["v"]["type"])

        # Expand heap children
        heap_ref = vars_by_name["heap"]["variablesReference"]
        self.assertGreater(heap_ref, 0)
        heap_children_resp = await self.variables(
            VariablesArguments(variables_reference=heap_ref)
        )
        self.assertTrue(heap_children_resp["success"])
        heap_children = {
            v["name"]: v for v in heap_children_resp["body"]["variables"]
        }
        self.assertIn("[0]", heap_children)
        self.assertIn("[1]", heap_children)
        self.assertIn("[2]", heap_children)
        self.assertIn("[3]", heap_children)
        self.assertIn("[4]", heap_children)

        elem0_ref = heap_children["[0]"]["variablesReference"]
        self.assertGreater(elem0_ref, 0)
        elem0_resp = await self.variables(
            VariablesArguments(variables_reference=elem0_ref)
        )
        self.assertTrue(elem0_resp["success"])
        elem0_fields = {
            v["name"]: v["value"] for v in elem0_resp["body"]["variables"]
        }
        self.assertEqual(elem0_fields["num"], "10")

        # Expand NestedVecs v fields and nested vector children
        v_ref = vars_by_name["v"]["variablesReference"]
        self.assertGreater(v_ref, 0)
        v_fields_resp = await self.variables(
            VariablesArguments(variables_reference=v_ref)
        )
        self.assertTrue(v_fields_resp["success"])
        v_fields = {v["name"]: v for v in v_fields_resp["body"]["variables"]}
        self.assertIn("input", v_fields)
        self.assertIn("output", v_fields)

        input_ref = v_fields["input"]["variablesReference"]
        self.assertGreater(input_ref, 0)
        input_children_resp = await self.variables(
            VariablesArguments(variables_reference=input_ref)
        )
        self.assertTrue(input_children_resp["success"])
        input_children = {
            v["name"]: v for v in input_children_resp["body"]["variables"]
        }
        self.assertIn("[0]", input_children)
        self.assertIn("[1]", input_children)
        self.assertIn("[2]", input_children)
        self.assertIn("[3]", input_children)

        input_elem0_ref = input_children["[0]"]["variablesReference"]
        self.assertGreater(input_elem0_ref, 0)
        input_elem0_resp = await self.variables(
            VariablesArguments(variables_reference=input_elem0_ref)
        )
        self.assertTrue(input_elem0_resp["success"])
        input_elem0_fields = {
            v["name"]: v["value"] for v in input_elem0_resp["body"]["variables"]
        }
        self.assertEqual(input_elem0_fields["num"], "1")


class TestLaunch(DapTestCase):
    async def test_strong_attach(self) -> None:
        self.launch(
            LaunchArguments(
                process="fuchsia-pkg://fuchsia.com/crasher#meta/cpp_crasher.cm"
            )
        )
        await self.on_event("stopped", 30.0).expect(
            {
                "body": {
                    "reason": "exception",
                }
            }
        )


class TestDapStackTrace(DapTestCase):
    # TODO(https://fxbug.dev/529615917): remove target_cpu requirement once core.vim3-vg-release is not flaky.
    require_build_type = ["target_cpu!=arm64", "is_coverage=false"]

    async def test_pretty_stack(self) -> None:
        self.launch(
            LaunchArguments(
                process="fuchsia-pkg://fuchsia.com/crasher#meta/rust_crasher.cm"
            )
        )

        stopped_event = await self.on_event("stopped", timeout=120.0)
        thread_id = stopped_event["body"]["threadId"]

        stack_resp = await self.zxdb_stack_trace(
            ZxdbStackTraceArguments(thread_id=thread_id, remote_unwind=True)
        )

        frames = stack_resp["body"]["stackFrames"]
        self.assertTrue(len(frames) > 0)

        # Find main frame and startup frame
        main_frame = next(
            (f for f in frames if "rust_crasher::main" in f["name"]), None
        )
        startup_frame = next(
            (
                f
                for f in frames
                if f.get("source", {}).get("origin") == "Rust startup"
            ),
            None,
        )

        self.assertIsNotNone(main_frame, "rust_crasher::main frame not found")
        self.assertIsNotNone(
            startup_frame, "'Rust startup' origin frame not found"
        )
        # Explicitly narrow optional union types for Mypy across dynamic assertIsNotNone calls.
        assert main_frame is not None
        assert startup_frame is not None

        # In console zxdb, we fold the frames. In DAP, we mark those frames as "subtle"
        # main frame should NOT be subtle
        self.assertNotEqual(main_frame.get("presentationHint"), "subtle")

        # startup frame SHOULD be subtle and have origin "Rust startup"
        self.assertEqual(startup_frame.get("presentationHint"), "subtle")
        self.assertEqual(
            startup_frame.get("source", {}).get("origin"), "Rust startup"
        )


class TestDapDataBreakpoints(DapTestCase):
    """End-to-end tests for DAP data breakpoint (write watchpoint) support."""

    require_build_type = ["optimize=none"]

    async def test_data_breakpoints(self) -> None:
        """Tests setting and triggering write watchpoints across two distinct phases:

        - Phase 1 (Global Variable): Resolves global variable `SomeGlobal` via
          `dataBreakpointInfo` from `main`, sets a write watchpoint, and verifies
          execution breaks in `LeafNoArgs()` when `SomeGlobal` is written to.
        - Phase 2 (Local Variable): Resolves local stack variable `lhs` via
          `dataBreakpointInfo` from `main`, sets a write watchpoint, and verifies
          execution breaks in `SwapPointedToValues()` when `*lhs` is modified.
        """
        cpp_functions_path = get_dap_source_path(
            "src/developer/debug/e2e_tests/inferiors/cpp_functions.cc"
        )
        # --- Phase 1: Global Variable Watchpoint (`SomeGlobal`) ---
        # Stop at line 60 (`NestedTwiceNoArgs();` in `main`) to test watching global `SomeGlobal`
        # (`SomeGlobal = i;` in `LeafNoArgs()` at line 16).
        bp_resp = await self.set_breakpoints(
            SetBreakpointsArguments(
                source=Source(path=cpp_functions_path),
                breakpoints=[
                    SourceBreakpoint(line=60),
                ],
            )
        )
        self.assertTrue(bp_resp["success"])
        bp_line_60_id = bp_resp["body"]["breakpoints"][0]["id"]

        self.launch(
            LaunchArguments(
                process="fuchsia-pkg://fuchsia.com/zxdb_e2e_inferiors#meta/cpp_functions.cm"
            )
        )
        stopped_event = await self.on_event("stopped", timeout=120.0)
        self.assertEqual(stopped_event["body"]["reason"], "breakpoint")
        self.assertIn(bp_line_60_id, stopped_event["body"]["hitBreakpointIds"])

        thread_id = stopped_event["body"]["threadId"]
        stack_resp = await self.zxdb_stack_trace(
            ZxdbStackTraceArguments(thread_id=thread_id, remote_unwind=True)
        )
        frames = stack_resp["body"]["stackFrames"]
        self.assertTrue(len(frames) > 0)
        main_frame_id = frames[0]["id"]

        # Query dataBreakpointInfo for global variable `SomeGlobal` and `::SomeGlobal` in `main`.
        global_info_resp = await self.data_breakpoint_info(
            DataBreakpointInfoArguments(
                name="SomeGlobal", frame_id=main_frame_id
            )
        )
        self.assertTrue(global_info_resp["success"])
        global_data_id = global_info_resp["body"]["dataId"]
        self.assertIsNotNone(global_data_id)

        qualified_global_info_resp = await self.data_breakpoint_info(
            DataBreakpointInfoArguments(
                name="::SomeGlobal", frame_id=main_frame_id
            )
        )
        self.assertTrue(qualified_global_info_resp["success"])
        self.assertEqual(
            qualified_global_info_resp["body"]["dataId"], global_data_id
        )

        # Install a write data breakpoint on `SomeGlobal`
        set_global_dbp_resp = await self.set_data_breakpoints(
            SetDataBreakpointsArguments(
                breakpoints=[
                    DataBreakpoint(data_id=global_data_id, access_type="write")
                ]
            )
        )
        self.assertTrue(set_global_dbp_resp["success"])
        self.assertEqual(len(set_global_dbp_resp["body"]["breakpoints"]), 1)
        self.assertTrue(
            set_global_dbp_resp["body"]["breakpoints"][0]["verified"]
        )
        global_watch_bp_id = set_global_dbp_resp["body"]["breakpoints"][0]["id"]

        # Resume execution; `SomeGlobal = i;` in `LeafNoArgs()` at line 16 must trigger the watchpoint.
        cont_resp = await self.continue_execution(
            ContinueArguments(thread_id=thread_id)
        )
        self.assertTrue(cont_resp["success"])

        global_watch_stopped = await self.on_event("stopped", timeout=120.0)
        self.assertEqual(
            global_watch_stopped["body"]["reason"], "data breakpoint"
        )
        self.assertIn(
            global_watch_bp_id, global_watch_stopped["body"]["hitBreakpointIds"]
        )

        global_watch_stack_resp = await self.zxdb_stack_trace(
            ZxdbStackTraceArguments(thread_id=thread_id, remote_unwind=True)
        )
        global_watch_frames = global_watch_stack_resp["body"]["stackFrames"]
        self.assertTrue(len(global_watch_frames) > 0)
        self.assertIn("LeafNoArgs", global_watch_frames[0]["name"])
        self.assertEqual(global_watch_frames[0]["line"], 17)

        # --- Phase 2: Local Variable Watchpoint (`lhs`) ---
        # 1. Clear the global watchpoint.
        clear_global_dbp_resp = await self.set_data_breakpoints(
            SetDataBreakpointsArguments(breakpoints=[])
        )
        self.assertTrue(clear_global_dbp_resp["success"])
        self.assertEqual(len(clear_global_dbp_resp["body"]["breakpoints"]), 0)

        # 2. Advance to line 69 in `main` where local `lhs` is initialized.
        set_bp_69_resp = await self.set_breakpoints(
            SetBreakpointsArguments(
                source=Source(path=cpp_functions_path),
                breakpoints=[SourceBreakpoint(line=69)],
            )
        )
        self.assertTrue(set_bp_69_resp["success"])
        bp_line_69_id = set_bp_69_resp["body"]["breakpoints"][0]["id"]

        cont_to_69_resp = await self.continue_execution(
            ContinueArguments(thread_id=thread_id)
        )
        self.assertTrue(cont_to_69_resp["success"])

        line_69_stopped = await self.on_event("stopped", timeout=120.0)
        self.assertEqual(line_69_stopped["body"]["reason"], "breakpoint")
        self.assertIn(
            bp_line_69_id, line_69_stopped["body"]["hitBreakpointIds"]
        )

        stack_resp = await self.zxdb_stack_trace(
            ZxdbStackTraceArguments(thread_id=thread_id, remote_unwind=True)
        )
        main_frame_id = stack_resp["body"]["stackFrames"][0]["id"]

        # Remove the source breakpoints so the next stop is strictly from the local watchpoint.
        clear_bp_resp = await self.set_breakpoints(
            SetBreakpointsArguments(
                source=Source(path=cpp_functions_path),
                breakpoints=[],
            )
        )
        self.assertTrue(clear_bp_resp["success"])

        # Query dataBreakpointInfo for local variable `lhs` in `main` and verify its dataId is
        # distinct from global `SomeGlobal`.
        info_resp = await self.data_breakpoint_info(
            DataBreakpointInfoArguments(name="lhs", frame_id=main_frame_id)
        )
        self.assertTrue(info_resp["success"])
        data_id = info_resp["body"]["dataId"]
        self.assertIsNotNone(data_id)
        self.assertNotEqual(data_id, global_data_id)
        self.assertIn("write", info_resp["body"]["accessTypes"])

        # Install a write data breakpoint on `lhs`
        set_dbp_resp = await self.set_data_breakpoints(
            SetDataBreakpointsArguments(
                breakpoints=[
                    DataBreakpoint(data_id=data_id, access_type="write")
                ]
            )
        )
        self.assertTrue(set_dbp_resp["success"])
        self.assertEqual(len(set_dbp_resp["body"]["breakpoints"]), 1)
        self.assertTrue(set_dbp_resp["body"]["breakpoints"][0]["verified"])
        watch_bp_id = set_dbp_resp["body"]["breakpoints"][0]["id"]

        # Resume execution; reads in AddIntPointers() and `int tmp = *lhs;` must not stop,
        # and the write `*lhs = *rhs;` at line 39 must trigger the data breakpoint.
        cont_resp = await self.continue_execution(
            ContinueArguments(thread_id=thread_id)
        )
        self.assertTrue(cont_resp["success"])

        watch_stopped = await self.on_event("stopped", timeout=120.0)
        self.assertEqual(watch_stopped["body"]["reason"], "data breakpoint")
        self.assertIn(watch_bp_id, watch_stopped["body"]["hitBreakpointIds"])

        # Verify we stopped in SwapPointedToValues at line 40 (right after `*lhs = *rhs;` at line 39)
        watch_stack_resp = await self.zxdb_stack_trace(
            ZxdbStackTraceArguments(thread_id=thread_id, remote_unwind=True)
        )
        watch_frames = watch_stack_resp["body"]["stackFrames"]
        self.assertTrue(len(watch_frames) > 0)
        self.assertIn("SwapPointedToValues", watch_frames[0]["name"])
        self.assertEqual(watch_frames[0]["line"], 40)

        # Clear data breakpoints and continue
        clear_dbp_resp = await self.set_data_breakpoints(
            SetDataBreakpointsArguments(breakpoints=[])
        )
        self.assertTrue(clear_dbp_resp["success"])
        self.assertEqual(len(clear_dbp_resp["body"]["breakpoints"]), 0)
        final_cont_resp = await self.continue_execution(
            ContinueArguments(thread_id=thread_id)
        )
        self.assertTrue(final_cont_resp["success"])


def main() -> None:
    parser = argparse.ArgumentParser()

    parser.add_argument(
        "--DAP_E2E_TESTS_FFX_TEST_DATA",  # The argument is capitalized to match the extra_args in BUILD.gn.
        help="the relative path from root_build_dir to the directory of ffx tools",
    )

    parser.add_argument(
        "--DAP_E2E_TESTS_SYMBOL_DIR",
        help="the relative path from root_build_dir to the inferior build-id symbol directory",
    )

    parser.add_argument(
        "--DAP_E2E_TESTS_BUILD_TYPE",
        help="the build_type string containing optimize/target_cpu/lto attributes",
    )

    parser.add_argument(
        "--dump-log",
        action="store_true",
        help="print DAP traffic history even if tests succeed",
    )

    args, unknown = parser.parse_known_args()

    if args.DAP_E2E_TESTS_FFX_TEST_DATA:
        os.environ[
            "DAP_E2E_TESTS_FFX_TEST_DATA"
        ] = args.DAP_E2E_TESTS_FFX_TEST_DATA

    if args.DAP_E2E_TESTS_SYMBOL_DIR:
        os.environ["DAP_E2E_TESTS_SYMBOL_DIR"] = args.DAP_E2E_TESTS_SYMBOL_DIR

    if args.DAP_E2E_TESTS_BUILD_TYPE:
        os.environ["DAP_E2E_TESTS_BUILD_TYPE"] = args.DAP_E2E_TESTS_BUILD_TYPE
        print(
            "BUILD_TYPE = ", os.environ["DAP_E2E_TESTS_BUILD_TYPE"], flush=True
        )

    if args.dump_log:
        os.environ["DAP_DUMP_LOG_ALWAYS"] = "1"

    # Reconstruct sys.argv for unittest.main so that the unittest.main won't complain
    sys.argv = [sys.argv[0]] + unknown
    unittest.main()


if __name__ == "__main__":
    main()
