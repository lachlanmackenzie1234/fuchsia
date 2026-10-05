# Copyright 2023 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.
"""Unit tests for honeydew.transports.serial_using_unix_socket.py."""

import socket
import unittest
from unittest import mock

from honeydew.transports.serial import errors as serial_errors
from honeydew.transports.serial import serial_using_unix_socket


class FastbootTests(unittest.TestCase):
    """Unit tests for honeydew.transports.serial_using_unix_socket.py."""

    def setUp(self) -> None:
        super().setUp()

        self.serial_obj: serial_using_unix_socket.SerialUsingUnixSocket = (
            serial_using_unix_socket.SerialUsingUnixSocket(
                device_name="device_name",
                socket_path="socket_path",
            )
        )

    @mock.patch.object(
        socket,
        "socket",
        autospec=True,
    )
    def test_read(self, mock_socket: mock.Mock) -> None:
        """Test case for serial_using_unix_socket.Socket.read()"""
        # Configure the mock to return a real byte string from recv().
        # The code under test will then call the real .decode() on this byte string.
        mock_socket.return_value.__enter__.return_value.recv.return_value = (
            b"test_data"
        )

        # Assert that the final result of the read() method is the decoded string.
        self.assertEqual(self.serial_obj.read(), "test_data")
        mock_socket.assert_called()

    @mock.patch.object(
        socket, "socket", autospec=True, side_effect=socket.error
    )
    def test_read_error(self, mock_socket: mock.Mock) -> None:
        """Test case for serial_using_unix_socket.Socket.read() raising exception"""
        with self.assertRaises(serial_errors.SerialError):
            self.serial_obj.read()
        mock_socket.assert_called()

    @mock.patch.object(
        socket,
        "socket",
        autospec=True,
    )
    def test_send(self, mock_socket: mock.Mock) -> None:
        """Test case for serial_using_unix_socket.Socket.send()"""
        self.serial_obj.send(cmd="echo hello")
        mock_socket.assert_called()

    @mock.patch.object(
        socket, "socket", autospec=True, side_effect=socket.error
    )
    def test_send_error(self, mock_socket: mock.Mock) -> None:
        """Test case for serial_using_unix_socket.Socket.send() raising exception"""
        with self.assertRaises(serial_errors.SerialError):
            self.serial_obj.send(cmd="echo hello")
        mock_socket.assert_called()

    @mock.patch.object(
        socket,
        "socket",
        autospec=True,
    )
    def test_send_and_recv(self, mock_socket: mock.Mock) -> None:
        """Test case for serial_using_unix_socket.Socket.send_and_recv()."""
        mock_ctx = mock_socket.return_value.__enter__.return_value
        mock_ctx.recv.side_effect = [
            b"$ echo hello\r\n",
            b"hello\r\n[DONE]\r\n",
        ]

        output = self.serial_obj.send_and_recv(
            cmd="echo hello",
            stop_tokens=("[DONE]",),
            timeout_sec=5.0,
        )
        self.assertEqual(output, "$ echo hello\r\nhello\r\n[DONE]\r\n")
        mock_socket.assert_called_once()
        mock_ctx.settimeout.assert_any_call(5.0)
        mock_ctx.connect.assert_called_once_with("socket_path")
        mock_ctx.sendall.assert_called_once()
        self.assertEqual(mock_ctx.recv.call_count, 2)

    @mock.patch.object(
        socket,
        "socket",
        autospec=True,
    )
    def test_send_and_recv_split_utf8_and_timeout(
        self, mock_socket: mock.Mock
    ) -> None:
        """Test case for send_and_recv() handling split UTF-8 and timeout."""
        mock_ctx = mock_socket.return_value.__enter__.return_value
        mock_ctx.recv.side_effect = [
            b"$ echo caf\xc3",
            b"\xa9\r\n",
            socket.timeout,
        ]

        output = self.serial_obj.send_and_recv(
            cmd="echo café",
            stop_tokens=("[DONE]",),
            timeout_sec=5.0,
        )
        self.assertEqual(output, "$ echo café\r\n")
        self.assertEqual(mock_ctx.recv.call_count, 3)

    @mock.patch.object(
        socket, "socket", autospec=True, side_effect=socket.error
    )
    def test_send_and_recv_error(self, mock_socket: mock.Mock) -> None:
        """Test case for send_and_recv() raising exception on socket error."""
        with self.assertRaises(serial_errors.SerialError):
            self.serial_obj.send_and_recv(cmd="echo hello")
        mock_socket.assert_called()

    @mock.patch.object(
        socket,
        "socket",
        autospec=True,
    )
    def test_send_and_recv_recv_error(self, mock_socket: mock.Mock) -> None:
        """Test case for send_and_recv() raising SerialError on recv error."""
        mock_ctx = mock_socket.return_value.__enter__.return_value
        mock_ctx.recv.side_effect = [
            b"partial output",
            socket.error("Connection reset"),
        ]

        with self.assertRaises(serial_errors.SerialError):
            self.serial_obj.send_and_recv(cmd="echo hello")
        self.assertEqual(mock_ctx.recv.call_count, 2)

    @mock.patch.object(
        serial_using_unix_socket._LOGGER,
        "debug",
        autospec=True,
    )
    @mock.patch.object(
        socket,
        "socket",
        autospec=True,
    )
    def test_send_and_recv_log_output(
        self, mock_socket: mock.Mock, mock_logger_debug: mock.Mock
    ) -> None:
        """Test case for send_and_recv() log_output parameter."""
        mock_ctx = mock_socket.return_value.__enter__.return_value
        mock_ctx.recv.side_effect = [
            b"$ echo hello\r\n",
            b"hello\r\n[DONE]\r\n",
        ]

        # By default log_output is True, so debug log should be emitted.
        output = self.serial_obj.send_and_recv(
            cmd="echo hello",
            stop_tokens=("[DONE]",),
        )
        self.assertEqual(output, "$ echo hello\r\nhello\r\n[DONE]\r\n")
        mock_logger_debug.assert_called_once_with(
            "Received output from serial: %s",
            "$ echo hello\r\nhello\r\n[DONE]\r\n",
        )

        mock_logger_debug.reset_mock()
        mock_ctx.recv.side_effect = [
            b"$ echo hello\r\n",
            b"hello\r\n[DONE]\r\n",
        ]
        # When log_output is False, debug log should not be emitted.
        output = self.serial_obj.send_and_recv(
            cmd="echo hello",
            stop_tokens=("[DONE]",),
            log_output=False,
        )
        self.assertEqual(output, "$ echo hello\r\nhello\r\n[DONE]\r\n")
        mock_logger_debug.assert_not_called()
