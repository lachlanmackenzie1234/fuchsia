# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

import asyncio
import io
import json
import unittest
from typing import Any

from pydap.client import DapClient, DapError
from pydap.dap_types import Source, SourceBreakpoint
from pydap.models import *


class MockWriter:
    def __init__(self) -> None:
        self.buffer = io.BytesIO()

    def write(self, data: bytes) -> None:
        self.buffer.write(data)

    async def drain(self) -> None:
        pass


def feed_dap_response(
    reader: asyncio.StreamReader, response: dict[str, Any]
) -> None:
    body = json.dumps(response, separators=(",", ":")).encode("utf-8")
    header = f"Content-Length: {len(body)}\r\n\r\n".encode("utf-8")
    reader.feed_data(header + body)


class TestDapClient(unittest.IsolatedAsyncioTestCase):
    def _start_client(self, client: DapClient) -> asyncio.StreamReader:
        reader = asyncio.StreamReader()
        event_queue: asyncio.Queue[dict[str, Any]] = asyncio.Queue()
        asyncio.create_task(client.run(reader, event_queue))
        return reader

    # shouldn't send request before run the client.
    async def test_not_running_error(self) -> None:
        client = DapClient()
        writer = MockWriter()
        with self.assertRaises(DapError) as cm:
            client._send_request_future(writer, "initialize")
        self.assertIn("DapClient is not running", str(cm.exception))

    async def test_run(self) -> None:
        client = DapClient()
        reader = asyncio.StreamReader()
        event_queue: asyncio.Queue[dict[str, Any]] = asyncio.Queue()

        run_task = asyncio.create_task(client.run(reader, event_queue))
        await asyncio.sleep(0)
        self.assertTrue(client.is_running)

        reader.feed_eof()
        await run_task
        self.assertFalse(client.is_running)

    async def test_read_message(self) -> None:
        data = b'Content-Length: 26\r\n\r\n{"seq":1,"type":"request"}'
        reader = asyncio.StreamReader()
        reader.feed_data(data)
        reader.feed_eof()

        client = DapClient()
        msg = await client._read_message(reader)
        self.assertIsNotNone(msg)
        assert msg is not None
        self.assertEqual(msg["seq"], 1)

        self.assertEqual(msg["type"], "request")

    async def test_write_message(self) -> None:
        client = DapClient()
        value = {"seq": 1, "type": "request"}

        writer = MockWriter()
        await client._write_message(writer, value)

        buffer_val = writer.buffer.getvalue()
        headers, body = buffer_val.split(b"\r\n\r\n", 1)
        self.assertEqual(headers, b"Content-Length: 26")
        self.assertEqual(json.loads(body.decode("utf-8")), value)

    async def test__send_request(self) -> None:
        client = DapClient()
        reader = self._start_client(client)

        writer = MockWriter()

        send_task = asyncio.create_task(
            client._send_request(writer, "initialize")
        )

        await asyncio.sleep(0)
        await client._write_queue.join()

        buffer_val = writer.buffer.getvalue()
        headers, body = buffer_val.split(b"\r\n\r\n", 1)
        req_val = json.loads(body.decode("utf-8"))
        seq = req_val["seq"]

        response = {
            "seq": 10,
            "type": "response",
            "request_seq": seq,
            "success": True,
            "command": "initialize",
        }

        feed_dap_response(reader, response)

        resp = await send_task
        self.assertTrue(resp["success"])
        self.assertEqual(resp["request_seq"], seq)

    async def test_initialize(self) -> None:
        client = DapClient()
        reader = self._start_client(client)

        writer = MockWriter()
        args = InitializeArguments(adapter_id="test")
        send_task = asyncio.create_task(client.initialize(writer, args))

        await asyncio.sleep(0)
        await client._write_queue.join()

        buffer_val = writer.buffer.getvalue()
        headers, body = buffer_val.split(b"\r\n\r\n", 1)
        req_val = json.loads(body.decode("utf-8"))
        seq = req_val["seq"]

        response = {
            "seq": 10,
            "type": "response",
            "request_seq": seq,
            "success": True,
            "command": "initialize",
        }

        feed_dap_response(reader, response)

        resp = await send_task
        self.assertTrue(resp.success)

    async def test_disconnect(self) -> None:
        client = DapClient()
        reader = self._start_client(client)

        writer = MockWriter()
        args = DisconnectArguments(terminate_debuggee=True)
        send_task = asyncio.create_task(client.disconnect(writer, args))

        await asyncio.sleep(0)
        await client._write_queue.join()

        buffer_val = writer.buffer.getvalue()
        headers, body = buffer_val.split(b"\r\n\r\n", 1)
        req_val = json.loads(body.decode("utf-8"))
        seq = req_val["seq"]

        response = {
            "seq": 10,
            "type": "response",
            "request_seq": seq,
            "success": True,
            "command": "disconnect",
        }

        feed_dap_response(reader, response)

        resp = await send_task
        self.assertTrue(resp.success)

    async def test_stack_trace(self) -> None:
        client = DapClient()
        reader = self._start_client(client)

        writer = MockWriter()
        args = StackTraceArguments(thread_id=1)
        send_task = asyncio.create_task(client.stack_trace(writer, args))

        await asyncio.sleep(0)
        await client._write_queue.join()

        buffer_val = writer.buffer.getvalue()
        headers, body = buffer_val.split(b"\r\n\r\n", 1)
        req_val = json.loads(body.decode("utf-8"))
        seq = req_val["seq"]

        response = {
            "seq": 10,
            "type": "response",
            "request_seq": seq,
            "success": True,
            "command": "stackTrace",
            "body": {"stackFrames": []},
        }

        feed_dap_response(reader, response)

        resp = await send_task
        self.assertEqual(resp.body.stack_frames, [])

    async def test_continue_thread(self) -> None:
        client = DapClient()
        reader = self._start_client(client)

        writer = MockWriter()
        args = ContinueArguments(thread_id=1)
        send_task = asyncio.create_task(client.continue_thread(writer, args))

        await asyncio.sleep(0)
        await client._write_queue.join()

        buffer_val = writer.buffer.getvalue()
        headers, body = buffer_val.split(b"\r\n\r\n", 1)
        req_val = json.loads(body.decode("utf-8"))
        seq = req_val["seq"]

        response = {
            "seq": 10,
            "type": "response",
            "request_seq": seq,
            "success": True,
            "command": "continue",
            "body": {"allThreadsContinued": True},
        }

        feed_dap_response(reader, response)

        resp = await send_task
        self.assertIsInstance(resp, ContinueResponse)
        self.assertTrue(resp.success)
        self.assertIsNotNone(resp.body)
        self.assertTrue(resp.body.all_threads_continued)

    def test_continue_response_empty_body(self) -> None:
        response = {
            "seq": 10,
            "type": "response",
            "request_seq": 1,
            "success": True,
            "command": "continue",
            "body": {},
        }
        resp = ContinueResponse.model_validate(response)
        self.assertIsInstance(resp, ContinueResponse)
        self.assertIsNotNone(resp.body)
        self.assertTrue(resp.body.all_threads_continued)

    def test_continue_response_missing_body(self) -> None:
        response = {
            "seq": 10,
            "type": "response",
            "request_seq": 1,
            "success": True,
            "command": "continue",
        }
        resp = ContinueResponse.model_validate(response)
        self.assertIsInstance(resp, ContinueResponse)
        self.assertIsNotNone(resp.body)
        self.assertTrue(resp.body.all_threads_continued)

    async def test_pause_thread(self) -> None:
        client = DapClient()
        reader = self._start_client(client)

        writer = MockWriter()
        args = PauseArguments(thread_id=1)
        send_task = asyncio.create_task(client.pause_thread(writer, args))

        await asyncio.sleep(0)
        await client._write_queue.join()

        buffer_val = writer.buffer.getvalue()
        headers, body = buffer_val.split(b"\r\n\r\n", 1)
        req_val = json.loads(body.decode("utf-8"))
        seq = req_val["seq"]

        response = {
            "seq": 10,
            "type": "response",
            "request_seq": seq,
            "success": True,
            "command": "pause",
        }

        feed_dap_response(reader, response)

        resp = await send_task
        self.assertTrue(resp.success)

    async def test_threads(self) -> None:
        client = DapClient()
        reader = self._start_client(client)

        writer = MockWriter()
        send_task = asyncio.create_task(client.threads(writer))

        await asyncio.sleep(0)
        await client._write_queue.join()

        buffer_val = writer.buffer.getvalue()
        headers, body = buffer_val.split(b"\r\n\r\n", 1)
        req_val = json.loads(body.decode("utf-8"))
        seq = req_val["seq"]

        response = {
            "seq": 10,
            "type": "response",
            "request_seq": seq,
            "success": True,
            "command": "threads",
            "body": {
                "threads": [
                    {"id": 1234, "name": "main"},
                    {"id": 5678, "name": "worker"},
                ]
            },
        }

        feed_dap_response(reader, response)

        resp = await send_task
        self.assertEqual(len(resp.body.threads), 2)
        self.assertEqual(resp.body.threads[0].id, 1234)
        self.assertEqual(resp.body.threads[0].name, "main")

    async def test_attach(self) -> None:
        client = DapClient()
        reader = self._start_client(client)

        writer = MockWriter()
        args = AttachRequestArguments(
            restart=True, extra_fields={"process": "my_process"}
        )
        send_task = asyncio.create_task(client.attach(writer, args))

        await asyncio.sleep(0)
        await client._write_queue.join()

        buffer_val = writer.buffer.getvalue()
        headers, body = buffer_val.split(b"\r\n\r\n", 1)
        req_val = json.loads(body.decode("utf-8"))
        seq = req_val["seq"]

        response = {
            "seq": 10,
            "type": "response",
            "request_seq": seq,
            "success": True,
            "command": "attach",
        }

        feed_dap_response(reader, response)

        resp = await send_task
        self.assertTrue(resp.success)
        self.assertTrue(req_val["arguments"]["__restart"])
        self.assertEqual(req_val["arguments"]["process"], "my_process")

    async def test_launch(self) -> None:
        client = DapClient()
        reader = self._start_client(client)
        writer = MockWriter()
        args = LaunchArguments(process="my_process", launch_command="run")
        send_task = asyncio.create_task(client.launch(writer, args))

        await asyncio.sleep(0)
        await client._write_queue.join()

        buffer_val = writer.buffer.getvalue()
        headers, body = buffer_val.split(b"\r\n\r\n", 1)
        req_val = json.loads(body.decode("utf-8"))
        seq = req_val["seq"]

        response = {
            "seq": 10,
            "type": "response",
            "request_seq": seq,
            "success": True,
            "command": "launch",
        }

        feed_dap_response(reader, response)

        resp = await send_task
        self.assertTrue(resp.success)
        self.assertEqual(req_val["arguments"]["process"], "my_process")
        self.assertEqual(req_val["arguments"]["launchCommand"], "run")

    async def test_evaluate(self) -> None:
        client = DapClient()
        reader = self._start_client(client)
        writer = MockWriter()
        args = EvaluateArguments(
            expression="1 + 1", context="repl", frame_id=42
        )
        send_task = asyncio.create_task(client.evaluate(writer, args))

        await asyncio.sleep(0)
        await client._write_queue.join()

        buffer_val = writer.buffer.getvalue()
        headers, body = buffer_val.split(b"\r\n\r\n", 1)
        req_val = json.loads(body.decode("utf-8"))
        seq = req_val["seq"]

        response = {
            "seq": 10,
            "type": "response",
            "request_seq": seq,
            "success": True,
            "command": "evaluate",
            "body": {
                "result": "2",
                "type": "int",
                "variablesReference": 0,
            },
        }

        feed_dap_response(reader, response)

        resp = await send_task
        self.assertTrue(resp.success)
        self.assertEqual(resp.body.result, "2")
        self.assertEqual(resp.body.type, "int")
        self.assertEqual(resp.body.variables_reference, 0)
        self.assertEqual(req_val["arguments"]["expression"], "1 + 1")
        self.assertEqual(req_val["arguments"]["context"], "repl")
        self.assertEqual(req_val["arguments"]["frameId"], 42)

    async def test_scopes(self) -> None:
        client = DapClient()
        reader = self._start_client(client)
        writer = MockWriter()
        args = ScopesArguments(frame_id=42)
        send_task = asyncio.create_task(client.scopes(writer, args))

        await asyncio.sleep(0)
        await client._write_queue.join()

        buffer_val = writer.buffer.getvalue()
        headers, body = buffer_val.split(b"\r\n\r\n", 1)
        req_val = json.loads(body.decode("utf-8"))
        seq = req_val["seq"]

        response = {
            "seq": 10,
            "type": "response",
            "request_seq": seq,
            "success": True,
            "command": "scopes",
            "body": {
                "scopes": [
                    {
                        "name": "Locals",
                        "variablesReference": 100,
                        "expensive": False,
                    }
                ]
            },
        }

        feed_dap_response(reader, response)

        resp = await send_task
        self.assertTrue(resp.success)
        self.assertEqual(len(resp.body.scopes), 1)
        self.assertEqual(resp.body.scopes[0].name, "Locals")
        self.assertEqual(resp.body.scopes[0].variables_reference, 100)
        self.assertFalse(resp.body.scopes[0].expensive)
        self.assertEqual(req_val["arguments"]["frameId"], 42)

    async def test_variables(self) -> None:
        client = DapClient()
        reader = self._start_client(client)
        writer = MockWriter()
        args = VariablesArguments(variables_reference=100)
        send_task = asyncio.create_task(client.variables(writer, args))

        await asyncio.sleep(0)
        await client._write_queue.join()

        buffer_val = writer.buffer.getvalue()
        headers, body = buffer_val.split(b"\r\n\r\n", 1)
        req_val = json.loads(body.decode("utf-8"))
        seq = req_val["seq"]

        response = {
            "seq": 10,
            "type": "response",
            "request_seq": seq,
            "success": True,
            "command": "variables",
            "body": {
                "variables": [
                    {
                        "name": "foo",
                        "value": "bar",
                        "variablesReference": 0,
                        "type": "str",
                    }
                ]
            },
        }

        feed_dap_response(reader, response)

        resp = await send_task
        self.assertTrue(resp.success)
        self.assertEqual(len(resp.body.variables), 1)
        self.assertEqual(resp.body.variables[0].name, "foo")
        self.assertEqual(resp.body.variables[0].value, "bar")
        self.assertEqual(resp.body.variables[0].variables_reference, 0)
        self.assertEqual(resp.body.variables[0].type, "str")
        self.assertEqual(req_val["arguments"]["variablesReference"], 100)
        self.assertNotIn("start", req_val["arguments"])
        self.assertNotIn("count", req_val["arguments"])

    async def test_variables_with_start_only(self) -> None:
        client = DapClient()
        reader = self._start_client(client)
        writer = MockWriter()
        args = VariablesArguments(variables_reference=100, start=5)
        send_task = asyncio.create_task(client.variables(writer, args))

        await asyncio.sleep(0)
        await client._write_queue.join()

        buffer_val = writer.buffer.getvalue()
        headers, body = buffer_val.split(b"\r\n\r\n", 1)
        req_val = json.loads(body.decode("utf-8"))
        seq = req_val["seq"]

        response = {
            "seq": 10,
            "type": "response",
            "request_seq": seq,
            "success": True,
            "command": "variables",
            "body": {"variables": []},
        }

        feed_dap_response(reader, response)

        resp = await send_task
        self.assertTrue(resp.success)
        self.assertEqual(req_val["arguments"]["variablesReference"], 100)
        self.assertEqual(req_val["arguments"]["start"], 5)
        self.assertNotIn("count", req_val["arguments"])

    async def test_variables_with_count_only(self) -> None:
        client = DapClient()
        reader = self._start_client(client)
        writer = MockWriter()
        args = VariablesArguments(variables_reference=100, count=10)
        send_task = asyncio.create_task(client.variables(writer, args))

        await asyncio.sleep(0)
        await client._write_queue.join()

        buffer_val = writer.buffer.getvalue()
        headers, body = buffer_val.split(b"\r\n\r\n", 1)
        req_val = json.loads(body.decode("utf-8"))
        seq = req_val["seq"]

        response = {
            "seq": 10,
            "type": "response",
            "request_seq": seq,
            "success": True,
            "command": "variables",
            "body": {"variables": []},
        }

        feed_dap_response(reader, response)

        resp = await send_task
        self.assertTrue(resp.success)
        self.assertEqual(req_val["arguments"]["variablesReference"], 100)
        self.assertNotIn("start", req_val["arguments"])
        self.assertEqual(req_val["arguments"]["count"], 10)

    async def test_variables_with_start_and_count(self) -> None:
        client = DapClient()
        reader = self._start_client(client)
        writer = MockWriter()
        args = VariablesArguments(variables_reference=100, start=5, count=10)
        send_task = asyncio.create_task(client.variables(writer, args))

        await asyncio.sleep(0)
        await client._write_queue.join()

        buffer_val = writer.buffer.getvalue()
        headers, body = buffer_val.split(b"\r\n\r\n", 1)
        req_val = json.loads(body.decode("utf-8"))
        seq = req_val["seq"]

        response = {
            "seq": 10,
            "type": "response",
            "request_seq": seq,
            "success": True,
            "command": "variables",
            "body": {"variables": []},
        }

        feed_dap_response(reader, response)

        resp = await send_task
        self.assertTrue(resp.success)
        self.assertEqual(req_val["arguments"]["variablesReference"], 100)
        self.assertEqual(req_val["arguments"]["start"], 5)
        self.assertEqual(req_val["arguments"]["count"], 10)

    async def test_set_breakpoints(self) -> None:
        client = DapClient()
        reader = self._start_client(client)
        writer = MockWriter()
        args = SetBreakpointsArguments(
            source=Source(path="/path/to/file.rs"),
            breakpoints=[SourceBreakpoint(line=12)],
        )
        send_task = asyncio.create_task(client.set_breakpoints(writer, args))

        await asyncio.sleep(0)
        await client._write_queue.join()

        buffer_val = writer.buffer.getvalue()
        headers, body = buffer_val.split(b"\r\n\r\n", 1)
        req_val = json.loads(body.decode("utf-8"))
        seq = req_val["seq"]

        response = {
            "seq": 10,
            "type": "response",
            "request_seq": seq,
            "success": True,
            "command": "setBreakpoints",
            "body": {
                "breakpoints": [
                    {
                        "id": 1,
                        "verified": True,
                        "source": {"path": "/path/to/file.rs"},
                        "line": 12,
                    }
                ]
            },
        }

        feed_dap_response(reader, response)

        resp = await send_task
        self.assertTrue(resp.success)
        self.assertEqual(len(resp.body.breakpoints), 1)
        self.assertEqual(resp.body.breakpoints[0].id, 1)
        self.assertTrue(resp.body.breakpoints[0].verified)
        self.assertEqual(resp.body.breakpoints[0].line, 12)
        self.assertEqual(
            req_val["arguments"]["source"]["path"], "/path/to/file.rs"
        )

    async def test_error_response(self) -> None:
        client = DapClient()
        reader = self._start_client(client)
        writer = MockWriter()
        args = SetBreakpointsArguments(source=Source(path="relative.rs"))

        send_task = asyncio.create_task(client.set_breakpoints(writer, args))
        await asyncio.sleep(0)
        await client._write_queue.join()
        buffer_val = writer.buffer.getvalue()
        headers, body = buffer_val.split(b"\r\n\r\n", 1)
        req_val = json.loads(body.decode("utf-8"))
        seq = req_val["seq"]

        response = {
            "seq": 10,
            "type": "response",
            "request_seq": seq,
            "success": False,
            "command": "setBreakpoints",
            "message": "SetBreakpointsRequest path must be absolute!",
        }

        feed_dap_response(reader, response)

        with self.assertRaises(DapError) as cm:
            await send_task
        self.assertIn(
            "SetBreakpointsRequest path must be absolute!", str(cm.exception)
        )
