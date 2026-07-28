# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

import asyncio
import json
import logging
from typing import Any, Protocol

from .dap_types import DapBaseModel
from .models import (
    AttachRequestArguments,
    ContinueArguments,
    ContinueResponse,
    DisconnectArguments,
    EvaluateArguments,
    EvaluateResponse,
    InitializeArguments,
    LaunchArguments,
    MessageType,
    PauseArguments,
    Response,
    ScopesArguments,
    ScopesResponse,
    SetBreakpointsArguments,
    SetBreakpointsResponse,
    StackTraceArguments,
    StackTraceResponse,
    ThreadsResponse,
    VariablesArguments,
    VariablesResponse,
)


class StreamWriterProtocol(Protocol):
    """Protocol for writer objects handling raw byte writes."""

    def write(self, data: bytes) -> None:
        ...

    async def drain(self) -> None:
        ...


logger = logging.getLogger(__name__)


class DapError(Exception):
    """Base exception for DAP client errors."""


class DapClient:
    """A client for the Debug Adapter Protocol."""

    DEFAULT_REQUEST_TIMEOUT: float = 5.0

    def __init__(self) -> None:
        """Initializes the DAP client."""
        self._pending_requests: dict[int, asyncio.Future[Any]] = {}
        self._seq_counter = 1
        self._write_queue: asyncio.Queue[
            tuple[
                StreamWriterProtocol,
                dict[str, Any],
                asyncio.Future[None],
                asyncio.Future[Any],
            ]
        ] = asyncio.Queue()
        self._reader_task: asyncio.Task[None] | None = None
        self._writer_task: asyncio.Task[None] | None = None

    @property
    def is_running(self) -> bool:
        """Returns True if both the reader loop and writer tasks are currently running."""
        return (
            self._reader_task is not None
            and not self._reader_task.done()
            and self._writer_task is not None
            and not self._writer_task.done()
        )

    async def close(self) -> None:
        """Closes the client and cancels active writer and reader tasks if running."""
        tasks_to_cancel: list[asyncio.Task[None]] = []
        if self._writer_task is not None and not self._writer_task.done():
            tasks_to_cancel.append(self._writer_task)
        self._writer_task = None

        curr_task = asyncio.current_task()
        if (
            self._reader_task is not None
            and not self._reader_task.done()
            and self._reader_task != curr_task
        ):
            tasks_to_cancel.append(self._reader_task)
        self._reader_task = None

        for task in tasks_to_cancel:
            task.cancel()
            try:
                await task
            except asyncio.CancelledError:
                pass

        while not self._write_queue.empty():
            try:
                _, _, sent_fut, data_fut = self._write_queue.get_nowait()
                if not sent_fut.done():
                    sent_fut.cancel()
                if not data_fut.done():
                    data_fut.cancel()
                self._write_queue.task_done()
            except asyncio.QueueEmpty:
                break

        for fut in self._pending_requests.values():
            if not fut.done():
                fut.cancel()
        self._pending_requests.clear()

    async def _run_writer_task(self) -> None:
        """Processes queued write requests sequentially in FIFO order."""
        while True:
            try:
                (
                    writer,
                    request,
                    sent_fut,
                    data_fut,
                ) = await self._write_queue.get()
            except asyncio.CancelledError:
                break

            # in case the caller gives up waiting for this future (e.g. cancel).
            if data_fut.done():
                if not sent_fut.done():
                    sent_fut.cancel()
                self._write_queue.task_done()
                continue

            try:
                await self._write_message(writer, request)
                if not sent_fut.done():
                    sent_fut.set_result(None)
            except Exception as e:
                seq = request.get("seq")
                if seq is not None:
                    self._pending_requests.pop(seq, None)
                if not sent_fut.done():
                    sent_fut.set_exception(e)
                if not data_fut.done():
                    data_fut.set_exception(e)
            finally:
                self._write_queue.task_done()

    # TODO(http://fxbug.dev/538056589) : let writer be provided in here instead of being provided through send_request every time.
    async def run(
        self, reader: asyncio.StreamReader, event_queue: asyncio.Queue[Any]
    ) -> None:
        """Runs the client's read loop, processing messages from the reader.

        Args:
            reader: Stream reader to receive messages from the debug adapter.
            event_queue: Queue to put received DAP events into.
        """
        self._reader_task = asyncio.current_task()
        self._writer_task = asyncio.create_task(self._run_writer_task())
        try:
            while True:
                msg = await self._read_message(reader)
                if msg is None:
                    break  # EOF

                msg_type = msg.get("type")
                if msg_type == MessageType.EVENT.value:
                    await event_queue.put(msg)
                elif msg_type == MessageType.RESPONSE.value:
                    req_seq = msg.get("request_seq")
                    if req_seq in self._pending_requests:
                        fut = self._pending_requests.pop(req_seq)
                        if not fut.done():
                            fut.set_result(msg)
        except Exception:
            logger.exception("Error in DAP client run loop")
        finally:
            await self.close()

    # To make requests execute `_write_message` by FIFO order, we rely on the run_writer_task.
    def _send_request_future(
        self,
        writer: StreamWriterProtocol,
        command: str,
        arguments: DapBaseModel | None = None,
    ) -> tuple[int, asyncio.Future[None], asyncio.Future[dict[str, Any]]]:
        """Sends a request to the debug adapter synchronously by queueing it for the write worker.

        Args:
            writer: Stream writer to send the request to.
            command: The DAP command name.
            arguments: Optional arguments for the command.

        Returns:
            A tuple of (sequence number, sent future, response future).

        Raises:
            DapError: If the client is not running.
            TypeError: If arguments is not a DapBaseModel instance.
        """
        if not self.is_running:
            raise DapError(
                "DapClient is not running. Call 'run()' before sending requests."
            )

        seq = self._seq_counter
        self._seq_counter += 1

        loop = asyncio.get_running_loop()
        sent_fut: asyncio.Future[None] = loop.create_future()
        data_fut: asyncio.Future[dict[str, Any]] = loop.create_future()

        request: dict[str, Any] = {
            "seq": seq,
            "type": MessageType.REQUEST.value,
            "command": command,
        }
        if arguments is not None:
            if not isinstance(arguments, DapBaseModel):
                raise TypeError(
                    f"arguments must be a DapBaseModel, got {type(arguments)}"
                )
            request["arguments"] = arguments.dump_dap()

        self._pending_requests[seq] = data_fut
        self._write_queue.put_nowait((writer, request, sent_fut, data_fut))

        return seq, sent_fut, data_fut

    async def _await_request_response(
        self,
        command: str,
        seq: int,
        sent_fut: asyncio.Future[None],
        data_fut: asyncio.Future[dict[str, Any]],
        timeout: float,
    ) -> dict[str, Any]:
        """Awaits wire transmission and response for a request with centralized cleanup."""
        try:
            async with asyncio.timeout(timeout):
                await sent_fut
                return await data_fut
        except TimeoutError:
            if not sent_fut.done():
                raise DapError(
                    f"CLIENT IO ERROR: Request {command} (seq={seq}) failed to write to socket within {timeout}s"
                )
            raise DapError(
                f"SERVER TIMEOUT: Request {command} (seq={seq}) sent over wire, but adapter timed out waiting for response after {timeout}s"
            )
        finally:
            self._pending_requests.pop(seq, None)
            if not sent_fut.done():
                sent_fut.cancel()
            if not data_fut.done():
                data_fut.cancel()

    async def _send_request(
        self,
        writer: StreamWriterProtocol,
        command: str,
        arguments: DapBaseModel | None = None,
        timeout: float = DEFAULT_REQUEST_TIMEOUT,
    ) -> dict[str, Any]:
        """Sends a request to the debug adapter and waits for the response.

        Args:
            writer: Stream writer to send the request to.
            command: The DAP command name.
            arguments: Optional arguments for the command.
            timeout: Maximum time to wait for response in seconds.

        Returns:
            The response message dictionary from the adapter.

        Raises:
            DapError: If the request times out or framing fails.
        """

        # Instead of directly calling _write_message here,
        # we leverage _send_request_future, so this function inherently sends requests in FIFO order.
        # FIFO may not be necessary for this function and its client, but we keep it for consistency.
        seq, sent_fut, data_fut = self._send_request_future(
            writer, command, arguments
        )
        resp = await self._await_request_response(
            command, seq, sent_fut, data_fut, timeout
        )
        if not resp.get("success", True):
            msg = resp.get("message", "Unknown DAP error")
            logger.error(f"DAP request {command} (seq={seq}) failed: {msg}")
            raise DapError(f"DAP request {command} failed: {msg}")
        return resp

    async def initialize(
        self, writer: StreamWriterProtocol, args: InitializeArguments
    ) -> Response:
        """Sends an initialize request.

        Args:
            writer: Stream writer to send the request to.
            args: Arguments for the initialize request.

        Returns:
            The response model.
        """
        resp = await self._send_request(writer, "initialize", args)
        return Response.model_validate(resp)

    async def disconnect(
        self, writer: StreamWriterProtocol, args: DisconnectArguments
    ) -> Response:
        """Sends a disconnect request.

        Args:
            writer: Stream writer to send the request to.
            args: Arguments for the disconnect request.

        Returns:
            The response model.
        """
        resp = await self._send_request(writer, "disconnect", args)
        return Response.model_validate(resp)

    async def stack_trace(
        self, writer: StreamWriterProtocol, args: StackTraceArguments
    ) -> StackTraceResponse:
        """Sends a stackTrace request.

        Args:
            writer: Stream writer to send the request to.
            args: Arguments for the stackTrace request.

        Returns:
            The stackTrace response model.
        """
        resp = await self._send_request(writer, "stackTrace", args)
        return StackTraceResponse.model_validate(resp)

    async def continue_thread(
        self, writer: StreamWriterProtocol, args: ContinueArguments
    ) -> ContinueResponse:
        """Sends a continue request.

        Args:
            writer: Stream writer to send the request to.
            args: Arguments for the continue request.

        Returns:
            The continue response model.
        """
        resp = await self._send_request(writer, "continue", args)
        return ContinueResponse.model_validate(resp)

    async def pause_thread(
        self, writer: StreamWriterProtocol, args: PauseArguments
    ) -> Response:
        """Sends a pause request.

        Args:
            writer: Stream writer to send the request to.
            args: Arguments for the pause request.

        Returns:
            The response model.
        """
        resp = await self._send_request(writer, "pause", args)
        return Response.model_validate(resp)

    async def threads(self, writer: StreamWriterProtocol) -> ThreadsResponse:
        """Sends a threads request.

        Args:
            writer: Stream writer to send the request to.

        Returns:
            The threads response model.
        """
        resp = await self._send_request(writer, "threads")
        return ThreadsResponse.model_validate(resp)

    async def attach(
        self, writer: StreamWriterProtocol, args: AttachRequestArguments
    ) -> Response:
        """Sends an attach request.

        Args:
            writer: Stream writer to send the request to.
            args: Arguments for the attach request.

        Returns:
            The response model.
        """
        resp = await self._send_request(writer, "attach", args)
        return Response.model_validate(resp)

    async def launch(
        self, writer: StreamWriterProtocol, args: LaunchArguments
    ) -> Response:
        """Sends a launch request.

        Args:
            writer: Stream writer to send the request to.
            args: Arguments for the launch request.

        Returns:
            The response model.
        """
        resp = await self._send_request(writer, "launch", args)
        return Response.model_validate(resp)

    async def evaluate(
        self, writer: StreamWriterProtocol, args: EvaluateArguments
    ) -> EvaluateResponse:
        """Sends an evaluate request.

        Args:
            writer: Stream writer to send the request to.
            args: Arguments for the evaluate request.

        Returns:
            The response model.
        """
        resp = await self._send_request(writer, "evaluate", args)
        return EvaluateResponse.model_validate(resp)

    async def scopes(
        self, writer: StreamWriterProtocol, args: ScopesArguments
    ) -> ScopesResponse:
        """Sends a scopes request.

        Args:
            writer: Stream writer to send the request to.
            args: Arguments for the scopes request.

        Returns:
            The scopes response model.
        """
        resp = await self._send_request(writer, "scopes", args)
        return ScopesResponse.model_validate(resp)

    async def variables(
        self, writer: StreamWriterProtocol, args: VariablesArguments
    ) -> VariablesResponse:
        """Sends a variables request.

        Args:
            writer: Stream writer to send the request to.
            args: Arguments for the variables request.

        Returns:
            The variables response model.
        """
        resp = await self._send_request(writer, "variables", args)
        return VariablesResponse.model_validate(resp)

    async def set_breakpoints(
        self, writer: StreamWriterProtocol, args: SetBreakpointsArguments
    ) -> SetBreakpointsResponse:
        """Sends a setBreakpoints request.

        Args:
            writer: Stream writer to send the request to.
            args: Arguments for the setBreakpoints request.

        Returns:
            The setBreakpoints response model.
        """
        resp = await self._send_request(writer, "setBreakpoints", args)
        return SetBreakpointsResponse.model_validate(resp)

    async def _read_message(
        self, reader: asyncio.StreamReader
    ) -> dict[str, Any] | None:
        """Reads a single message from the reader, handling protocol framing.

        Args:
            reader: Stream reader to read from.

        Returns:
            The parsed message dictionary, or None on EOF.

        Raises:
            DapError: If framing headers are invalid or missing.
        """
        content_length = None
        while True:
            line = await reader.readline()
            if not line:
                return None  # EOF
            trimmed = line.decode("utf-8").strip()
            if not trimmed:
                break  # End of headers

            if trimmed.startswith("Content-Length:"):
                parts = trimmed.split(":")
                if len(parts) >= 2:
                    try:
                        content_length = int(parts[1].strip())
                    except ValueError:
                        raise DapError(f"Invalid Content-Length: {trimmed}")

        if content_length is None:
            raise DapError("Missing Content-Length header")

        body = await reader.readexactly(content_length)
        return json.loads(body.decode("utf-8"))

    async def _write_message(
        self, writer: StreamWriterProtocol, value: dict[str, Any]
    ) -> None:
        """Writes a message to the writer, handling protocol framing.

        Args:
            writer: Stream writer to write to.
            value: The message dictionary to serialize and send.
        """
        content = json.dumps(value, separators=(",", ":")).encode("utf-8")
        header = f"Content-Length: {len(content)}\r\n\r\n".encode("utf-8")
        writer.write(header)
        writer.write(content)
        await writer.drain()
