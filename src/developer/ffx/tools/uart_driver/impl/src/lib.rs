// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! Core daemon implementation for the Fuchsia UART host driver.
//!
//! Coordinates bidirectional multiplexing and reliable transport over a single UART link
//! using four dedicated asynchronous tasks:
//! * `coordinator_task`: Manages client connection lifecycle, channel allocations, and event routing.
//! * `sender_task`: Implements sliding-window Go-Back-N retransmission using [`uart_fpl::ResendSender`].
//! * `writer_task`: Transmits outgoing frames and cumulative ACKs to the UART device with batching and rate limiting.
//! * `receiver_task`: Demultiplexes incoming frames using [`uart_fpl::ResendReceiver`] and generates cumulative ACKs.

use ffx_tool_uart::DaemonMetrics;
use futures::channel::mpsc;
use futures::future::{FutureExt, poll_fn};
use futures::{SinkExt, StreamExt};
use std::collections::{HashMap, VecDeque};
use std::num::NonZeroU32;
use std::os::unix::fs::FileTypeExt as _;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{UnixListener, UnixStream};
use uart_driver_api::{ConnectionError, ConnectionMetadata, ConnectionStatus, UartProtocol};
use uart_fpl::{
    AckOutcome, AckTracker, CONTROL_CHANNEL_ID, DEFAULT_RETRANSMISSION_TIMEOUT, Frame, FrameParser,
    FrameStatus, FrameType, ResendReceiver, ResendSender, encode_frame,
};

pub const DEFAULT_CLIENT_DATA_CAPACITY: usize = 64;
const UART_READ_BUFFER_SIZE: usize = 16 * 1024;
const TERMINATE_POLL_RETRIES: usize = 5;
const TERMINATE_POLL_INTERVAL: Duration = Duration::from_millis(100);
const CLIENT_WRITER_CHANNEL_CAPACITY: usize = 256;
const MAX_UART_WRITE_BATCH_BYTES: usize = 4096;
const METADATA_CHECK_INTERVAL: Duration = Duration::from_secs(1);

pub fn is_char_device(path: &str) -> bool {
    std::fs::metadata(path).map(|m| m.file_type().is_char_device()).unwrap_or(false)
}

pub use ffx_tool_uart::stream::{AsyncUart, UartStream, connect_uart_stream};

/// Errors that can occur when managing Unix domain socket lifecycle and stale socket cleanup.
#[derive(thiserror::Error, Debug)]
pub enum RemoveAndBindError {
    /// The target socket path already exists and an active daemon is actively listening.
    #[error("Socket {0} already exists and is in use")]
    InUse(PathBuf),
    /// A stale socket file was detected but could not be removed from the filesystem.
    #[error("Could not remove stale socket at {0}: {1}")]
    RemoveStale(PathBuf, #[source] std::io::Error),
    /// An unexpected I/O error occurred while probing whether a pre-existing socket is active.
    #[error("Unexpected error when checking for stale socket at {0}: {1}")]
    ConnectCheck(PathBuf, #[source] std::io::Error),
    /// Binding the Unix domain listener to the target path failed.
    #[error("Could not listen on socket at {0}: {1}")]
    Bind(PathBuf, #[source] std::io::Error),
}

/// Errors that can occur when reading frames from a UART byte stream.
#[derive(thiserror::Error, Debug)]
pub enum ReaderError {
    /// Reading from the underlying UART stream failed with an I/O error.
    #[error("Failed to read from UART: {0}")]
    Read(#[source] std::io::Error),
    /// Reached end-of-file unexpectedly while reading from UART.
    #[error("EOF from UART")]
    Eof,
}

/// Errors that can occur during [`writer_task`] execution.
#[derive(thiserror::Error, Debug)]
pub enum WriterTaskError {
    /// Writing frame bytes to the UART device failed.
    #[error("Failed to write to UART: {0}")]
    Write(#[source] std::io::Error),
    /// Flushing the UART device stream failed.
    #[error("Failed to flush UART: {0}")]
    Flush(#[source] std::io::Error),
    /// Encoding an outgoing cumulative ACK frame failed.
    #[error("Failed to encode ACK frame: {0}")]
    EncodeAck(#[source] uart_fpl::FrameError),
}

/// Errors that can occur during [`receiver_task`] execution.
#[derive(thiserror::Error, Debug)]
pub enum ReceiverTaskError {
    /// Reading a frame from the UART stream failed.
    #[error("Reader error: {0}")]
    Reader(#[from] ReaderError),
    /// The target sent a protocol reset frame.
    #[error("Target requested protocol reset")]
    TargetRequestedReset,
    /// Advancing the sliding-window in-order sequence failed.
    #[error("Failed to advance in-order sequence: {0}")]
    AdvanceSeq(#[source] uart_fpl::UnexpectedSeqError),
    /// Enqueuing an ACK event sequence to the sender task failed.
    #[error("Failed to enqueue ACK event: {0}")]
    EnqueueAck(#[source] mpsc::SendError),
    /// Routing a received event to the coordinator failed because the coordinator channel closed.
    #[error("Failed to send {0} to coordinator: {1}")]
    CoordinatorClosed(&'static str, String),
}

/// Errors that can occur during [`sender_task`] execution.
#[derive(thiserror::Error, Debug)]
pub enum SenderTaskError {
    /// The ACK notification channel was unexpectedly closed.
    #[error("ACK channel closed")]
    AckChannelClosed,
    /// Retransmission attempts exceeded the configured limit during timeout handling.
    #[error("Retransmission limit exceeded: {0}")]
    RetransmissionLimit(#[from] uart_fpl::RetransmissionLimitExceeded),
    /// Encoding an outgoing frame failed.
    #[error("Failed to encode frame: {0}")]
    EncodeFrame(#[source] uart_fpl::FrameError),
    /// Sending a frame to the writer task failed because the writer channel closed.
    #[error("Failed to send frame to writer")]
    WriterClosed,
}

/// Unified error type encompassing failures across all asynchronous UART driver session tasks.
#[derive(thiserror::Error, Debug)]
pub enum TaskError {
    /// An error occurred in the writer task.
    #[error(transparent)]
    Writer(#[from] WriterTaskError),
    /// An error occurred in the receiver task.
    #[error(transparent)]
    Receiver(#[from] ReceiverTaskError),
    /// An error occurred in the sender task.
    #[error(transparent)]
    Sender(#[from] SenderTaskError),
}

async fn terminate_process(pid: u32) {
    if pid <= 1 || pid > i32::MAX as u32 {
        return;
    }
    // SAFETY: libc::kill is a standard POSIX system call to send SIGTERM for graceful termination.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
    for _ in 0..TERMINATE_POLL_RETRIES {
        tokio::time::sleep(TERMINATE_POLL_INTERVAL).await;
        if !ffx_tool_uart::is_running(pid) {
            log::info!("Orphaned daemon {} exited after SIGTERM.", pid);
            return;
        }
    }
    log::warn!("Orphaned daemon {} still running after SIGTERM. Sending SIGKILL...", pid);
    // SAFETY: libc::kill with SIGKILL forcefully terminates an unresponsive orphaned process.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
}

async fn check_and_kill_orphaned_daemon(socket_path: &std::path::Path, verify_daemon: bool) {
    let json_path = socket_path.with_extension("json");
    if !json_path.exists() {
        return;
    }
    let content = match std::fs::read_to_string(&json_path) {
        Ok(c) => c,
        Err(_) => return,
    };
    let metadata: ConnectionMetadata = match serde_json::from_str(&content) {
        Ok(m) => m,
        Err(_) => return,
    };
    let old_pid = metadata.pid;
    if old_pid == 0 {
        return;
    }
    let should_kill = if verify_daemon {
        ffx_tool_uart::is_driver_running(old_pid)
    } else {
        ffx_tool_uart::is_running(old_pid)
    };
    if should_kill {
        log::warn!(
            "Detected orphaned daemon with PID {} for target '{}' (socket is dead). Killing it...",
            old_pid,
            metadata.target
        );
        terminate_process(old_pid).await;
    }
}

/// Binds a [`UnixListener`] to `socket_path`, safely detecting and cleaning up stale socket files.
///
/// Attempts to connect to `socket_path` to verify whether an active daemon is running:
/// * If connection succeeds, the socket is actively in use, returning [`RemoveAndBindError::InUse`].
/// * If connection is refused, the socket is stale. If an associated metadata file indicates
///   an orphaned process, terminates the orphan and unlinks the stale socket before binding.
///
/// # Arguments
///
/// * `socket_path` - Filesystem path where the UNIX domain socket should be bound.
/// * `verify_daemon` - If `true`, verifies that any process recorded in the metadata file is an
///   actual `ffx-uart-driver` process before terminating it.
///
/// # Returns
///
/// The newly bound [`UnixListener`].
///
/// # Errors
///
/// Returns [`RemoveAndBindError`] if the socket is in active use, if removing a stale socket fails,
/// or if binding the listener fails.
pub async fn remove_and_bind_socket(
    socket_path: PathBuf,
    verify_daemon: bool,
) -> Result<UnixListener, RemoveAndBindError> {
    match UnixStream::connect(&socket_path).await {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            check_and_kill_orphaned_daemon(&socket_path, verify_daemon).await;
        }
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
            check_and_kill_orphaned_daemon(&socket_path, verify_daemon).await;
            if let Err(e) = std::fs::remove_file(&socket_path) {
                return Err(RemoveAndBindError::RemoveStale(socket_path, e));
            }
        }
        Ok(_) => {
            return Err(RemoveAndBindError::InUse(socket_path));
        }
        Err(e) => {
            return Err(RemoveAndBindError::ConnectCheck(socket_path, e));
        }
    }

    match UnixListener::bind(&socket_path) {
        Ok(s) => Ok(s),
        Err(e) => Err(RemoveAndBindError::Bind(socket_path, e)),
    }
}

/// Events emitted by client channel readers/writers or the UNIX domain listener.
pub enum ClientEvent {
    /// A new client connected on `channel_id`.
    NewChannel { channel_id: u16, stream: UnixStream },
    /// Incoming data read from a client socket on `channel_id`.
    ChannelData { channel_id: u16, data: Vec<u8> },
    /// A client socket closed or encountered an EOF/error on `channel_id`.
    ChannelClose { channel_id: u16 },
}

/// Events emitted by the UART session reader or supervisor loop.
#[derive(Debug)]
pub enum UartEvent {
    /// Decoded data payload received from the UART link for `channel_id`.
    UartData { channel_id: u16, data: Vec<u8> },
    /// Remote close frame received from the UART link for `channel_id`.
    UartClose { channel_id: u16 },
    /// A new UART session was established with an outgoing `sender_tx` queue.
    UpdateSender { sender_tx: mpsc::Sender<SenderMessage> },
    /// The active UART session disconnected or failed.
    UartDown,
}

impl PartialEq for UartEvent {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::UartData { channel_id: c1, data: d1 },
                Self::UartData { channel_id: c2, data: d2 },
            ) => c1 == c2 && d1 == d2,
            (Self::UartClose { channel_id: c1 }, Self::UartClose { channel_id: c2 }) => c1 == c2,
            (Self::UartDown, Self::UartDown) => true,
            _ => false,
        }
    }
}

/// Outgoing messages queued for transmission by [`sender_task`].
#[derive(Clone, Debug)]
pub enum SenderMessage {
    /// Data payload to frame and transmit on `channel_id`.
    Data { channel_id: u16, payload: Vec<u8> },
    /// Close notification frame to transmit on `channel_id`.
    Close { channel_id: u16 },
}

struct ClientState {
    writer_tx: mpsc::Sender<Vec<u8>>,
    _reader_task: fuchsia_async::Task<()>,
    _writer_task: fuchsia_async::Task<()>,
}

async fn dispatch_client_chunks(
    channel_id: u16,
    data: &[u8],
    coordinator_tx: &mut mpsc::Sender<ClientEvent>,
) -> bool {
    for chunk in data.chunks(uart_fpl::MAX_PAYLOAD_SIZE) {
        if coordinator_tx
            .send(ClientEvent::ChannelData { channel_id, data: chunk.to_vec() })
            .await
            .is_err()
        {
            return false;
        }
    }
    true
}

async fn client_reader_task(
    channel_id: u16,
    mut reader: OwnedReadHalf,
    mut coordinator_tx: mpsc::Sender<ClientEvent>,
) {
    let mut buf = [0; 1024];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => {
                // Drop the socket read half before awaiting `coordinator_tx.send` so the OS
                // descriptor is released immediately even if `coordinator_tx` is temporarily
                // backpressured.
                drop(reader);
                let _ = coordinator_tx.send(ClientEvent::ChannelClose { channel_id }).await;
                break;
            }
            Ok(n) => {
                log::trace!("CLIENT READ: channel={}, len={}", channel_id, n);
                if !dispatch_client_chunks(channel_id, &buf[..n], &mut coordinator_tx).await {
                    break;
                }
            }
        }
    }
}

const MAX_CLIENT_WRITE_BATCH_BYTES: usize = 64 * 1024;

async fn client_writer_task(
    channel_id: u16,
    mut writer: OwnedWriteHalf,
    mut receiver: mpsc::Receiver<Vec<u8>>,
    mut coordinator_tx: mpsc::Sender<ClientEvent>,
) {
    while let Some(data) = receiver.next().await {
        let batch = collect_data_batch(data, &mut receiver, MAX_CLIENT_WRITE_BATCH_BYTES);
        if let Err(e) = writer.write_all(&batch).await {
            log::error!("Client {} writer failed: {:?}", channel_id, e);
            break;
        }
        if let Err(e) = writer.flush().await {
            log::error!("Client {} writer flush failed: {:?}", channel_id, e);
            break;
        }
    }
    log::debug!("Client {} writer task exiting", channel_id);
    // Explicitly drop `receiver` (`writer_rx`) and `writer` BEFORE awaiting `coordinator_tx.send`.
    // If the client socket breaks while both `state.writer_tx` (coordinator -> client_writer_task)
    // and `coordinator_tx` (client_writer_task -> coordinator) are simultaneously full, dropping
    // `receiver` first causes `state.writer_tx.send(data)` in `coordinator_task` to immediately
    // wake with a disconnected error, breaking the two-task wait cycle.
    drop(receiver);
    drop(writer);
    let _ = coordinator_tx.send(ClientEvent::ChannelClose { channel_id }).await;
}

fn spawn_client_channel(
    channel_id: u16,
    stream: UnixStream,
    client_tx: &mpsc::Sender<ClientEvent>,
) -> ClientState {
    let (reader, writer) = stream.into_split();
    let reader_task =
        fuchsia_async::Task::local(client_reader_task(channel_id, reader, client_tx.clone()));
    let (writer_tx, writer_rx) = mpsc::channel(CLIENT_WRITER_CHANNEL_CAPACITY);
    let writer_task = fuchsia_async::Task::local(client_writer_task(
        channel_id,
        writer,
        writer_rx,
        client_tx.clone(),
    ));
    ClientState { writer_tx, _reader_task: reader_task, _writer_task: writer_task }
}

async fn handle_uart_event(
    event: UartEvent,
    active_channels: &mut HashMap<u16, ClientState>,
    outgoing_queue: &mut VecDeque<SenderMessage>,
    sender_tx: &mut Option<mpsc::Sender<SenderMessage>>,
) {
    match event {
        UartEvent::UartData { channel_id, data } => {
            if let Some(state) = active_channels.get_mut(&channel_id) {
                // Backpressure & Deadlock Design:
                // Fast path: try_send handles uncontested channels synchronously with zero future allocation.
                // When full (e.g. at 1,000,000 baud with a slow consumer), awaiting state.writer_tx.send
                // intentionally pauses coordinator_task from draining serial_rx so receiver_task withholds
                // sequence advancement and throttles the target via Go-Back-N without dropping the socket.
                // While waiting, we continue polling poll_send_outgoing so outgoing_queue keeps draining.
                match state.writer_tx.try_send(data) {
                    Ok(()) => {}
                    Err(e) if e.is_disconnected() => {
                        log::warn!("Client {} writer disconnected", channel_id);
                        active_channels.remove(&channel_id);
                        outgoing_queue.push_back(SenderMessage::Close { channel_id });
                    }
                    Err(e) => {
                        let mut send_fut =
                            std::pin::pin!(state.writer_tx.send(e.into_inner()).fuse());
                        loop {
                            let sender_ready_fut = futures::future::poll_fn(|cx| {
                                poll_send_outgoing(outgoing_queue, sender_tx, cx)
                            });
                            futures::select_biased! {
                                _ = sender_ready_fut.fuse() => {}
                                res = send_fut => {
                                    if let Err(e) = res {
                                        log::warn!("Client {} writer disconnected: {:?}", channel_id, e);
                                        active_channels.remove(&channel_id);
                                        outgoing_queue.push_back(SenderMessage::Close { channel_id });
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
        UartEvent::UartClose { channel_id } => {
            active_channels.remove(&channel_id);
        }
        UartEvent::UpdateSender { sender_tx: new_tx } => {
            *sender_tx = Some(new_tx);
        }
        UartEvent::UartDown => {
            log::warn!("UART connection went down, disconnecting all clients");
            active_channels.clear();
            outgoing_queue.clear();
            *sender_tx = None;
        }
    }
}

fn handle_client_event(
    event: ClientEvent,
    active_channels: &mut HashMap<u16, ClientState>,
    outgoing_queue: &mut VecDeque<SenderMessage>,
    sender_tx: &Option<mpsc::Sender<SenderMessage>>,
    client_tx: &mpsc::Sender<ClientEvent>,
) {
    match event {
        ClientEvent::NewChannel { channel_id, stream } => {
            if sender_tx.is_none() {
                log::warn!("Rejecting new channel {} because serial is down", channel_id);
                return;
            }
            let state = spawn_client_channel(channel_id, stream, client_tx);
            active_channels.insert(channel_id, state);
        }
        ClientEvent::ChannelData { channel_id, data } => {
            log::trace!("COORDINATOR: Client data channel={}, len={}", channel_id, data.len());
            outgoing_queue.push_back(SenderMessage::Data { channel_id, payload: data });
        }
        ClientEvent::ChannelClose { channel_id } => {
            if active_channels.remove(&channel_id).is_some() {
                outgoing_queue.push_back(SenderMessage::Close { channel_id });
            }
        }
    }
}

fn poll_send_outgoing(
    outgoing_queue: &mut VecDeque<SenderMessage>,
    sender_tx: &mut Option<mpsc::Sender<SenderMessage>>,
    cx: &mut std::task::Context<'_>,
) -> std::task::Poll<()> {
    if outgoing_queue.is_empty() {
        return std::task::Poll::Pending;
    }
    if let Some(tx) = sender_tx.as_mut() {
        match futures::sink::Sink::poll_ready(Pin::new(tx), cx) {
            std::task::Poll::Ready(Ok(())) => {
                let msg = outgoing_queue.pop_front().unwrap();
                let _ = futures::sink::Sink::start_send(Pin::new(tx), msg);
                std::task::Poll::Ready(())
            }
            std::task::Poll::Ready(Err(_)) => {
                outgoing_queue.clear();
                *sender_tx = None;
                std::task::Poll::Ready(())
            }
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    } else {
        std::task::Poll::Pending
    }
}

/// Coordinates client UNIX socket channels, backpressure, and event multiplexing with the active UART link.
pub async fn coordinator_task(
    mut client_rx: mpsc::Receiver<ClientEvent>,
    mut serial_rx: mpsc::Receiver<UartEvent>,
    client_tx: mpsc::Sender<ClientEvent>,
    mut sender_tx: Option<mpsc::Sender<SenderMessage>>,
    metrics: Arc<Mutex<DaemonMetrics>>,
) -> Result<(), TaskError> {
    let mut active_channels = HashMap::<u16, ClientState>::new();
    let mut outgoing_queue = VecDeque::<SenderMessage>::new();
    loop {
        let q_len = outgoing_queue.len();
        // Host-to-Target Backpressure: Stop polling `client_rx` once `outgoing_queue` reaches
        // `DEFAULT_CLIENT_DATA_CAPACITY` so `client_reader_task` instances suspend on
        // `coordinator_tx.send(...)` and apply OS socket backpressure to fast host clients
        // when the UART TX window is saturated.
        let mut client_rx_fut = std::pin::pin!(
            if q_len < DEFAULT_CLIENT_DATA_CAPACITY {
                futures::future::Either::Left(client_rx.next())
            } else {
                futures::future::Either::Right(futures::future::pending())
            }
            .fuse()
        );
        let sender_ready_fut = futures::future::poll_fn(|cx| {
            poll_send_outgoing(&mut outgoing_queue, &mut sender_tx, cx)
        });
        // Use fair `futures::select!` rather than `select_biased!` so continuous high-baudrate
        // incoming UART frames on `serial_rx` cannot starve `sender_ready_fut` or `client_rx_fut`.
        futures::select! {
            event = serial_rx.next().fuse() => {
                let Some(event) = event else { break };
                handle_uart_event(event, &mut active_channels, &mut outgoing_queue, &mut sender_tx).await;
            }
            _ = sender_ready_fut.fuse() => {}
            event = client_rx_fut => {
                let Some(event) = event else { break };
                handle_client_event(event, &mut active_channels, &mut outgoing_queue, &sender_tx, &client_tx);
            }
        }
        metrics.lock().unwrap().outgoing_queue_len = outgoing_queue.len() as u32;
    }
    Ok(())
}

fn current_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

struct DeviceWriter<W> {
    device: W,
    rate_limit: Option<u32>,
    next_allowed_time: std::time::Instant,
    metrics: Arc<Mutex<DaemonMetrics>>,
}

impl<W: AsyncWrite + Unpin> DeviceWriter<W> {
    async fn write_frame(&mut self, frame: &[u8]) -> Result<(), WriterTaskError> {
        if let Some(limit) = self.rate_limit.filter(|&l| l > 0) {
            let now = std::time::Instant::now();
            if self.next_allowed_time > now {
                fuchsia_async::Timer::new(self.next_allowed_time - now).await;
            }
            let frame_duration =
                std::time::Duration::from_secs_f64(frame.len() as f64 / limit as f64);
            self.next_allowed_time = std::cmp::max(now, self.next_allowed_time) + frame_duration;
        }
        log::trace!("WRITER: writing frame len={}", frame.len());
        self.device.write_all(frame).await.map_err(WriterTaskError::Write)?;
        self.device.flush().await.map_err(WriterTaskError::Flush)?;
        self.metrics.lock().unwrap().last_write_timestamp_ms = current_epoch_ms();
        Ok(())
    }

    async fn send_ack(&mut self, sid: u32, seq: u8) -> Result<(), WriterTaskError> {
        let ack_frame = encode_frame(sid, CONTROL_CHANNEL_ID, seq, FrameType::Ack, &[])
            .map_err(WriterTaskError::EncodeAck)?;
        self.write_frame(&ack_frame).await
    }
}

fn collect_data_batch(
    first_frame: Vec<u8>,
    rx: &mut mpsc::Receiver<Vec<u8>>,
    max_bytes: usize,
) -> Vec<u8> {
    let mut batch = first_frame;
    while let Ok(next_frame) = rx.try_recv() {
        batch.extend_from_slice(&next_frame);
        if batch.len() >= max_bytes {
            break;
        }
    }
    batch
}

/// Transmits outgoing data frames and cumulative ACKs to the UART device with batching and rate limiting.
pub async fn writer_task<W: AsyncWrite + Unpin>(
    device: W,
    ack_tracker: AckTracker,
    mut data_rx: mpsc::Receiver<Vec<u8>>,
    metrics: Arc<Mutex<DaemonMetrics>>,
    rate_limit: Option<u32>,
) -> Result<(), TaskError> {
    let mut writer =
        DeviceWriter { device, rate_limit, next_allowed_time: std::time::Instant::now(), metrics };
    loop {
        // Prioritize cumulative ACKs ahead of outgoing data batches so the remote Go-Back-N
        // sender receives timely window advances (or duplicate-ACK gap notifications after a
        // data glitch) without queuing behind bulk data frames.
        if let Some((sid, seq)) = ack_tracker.take_ack() {
            writer.send_ack(sid, seq).await.map_err(TaskError::Writer)?;
            continue;
        }
        if futures::stream::FusedStream::is_terminated(&data_rx) {
            break;
        }
        futures::select_biased! {
            (sid, seq) = ack_tracker.wait_ack().fuse() => {
                writer.send_ack(sid, seq).await.map_err(TaskError::Writer)?;
            }
            frame = data_rx.next() => {
                let Some(frame) = frame else { break };
                let batch = collect_data_batch(frame, &mut data_rx, MAX_UART_WRITE_BATCH_BYTES);
                writer.write_frame(&batch).await.map_err(TaskError::Writer)?;
            }
        }
    }
    Ok(())
}

/// Buffered asynchronous stream reader that extracts framed [`Frame`] packets from a UART byte stream.
pub struct UartReader<R: AsyncRead + Unpin> {
    client: R,
    parser: FrameParser,
    buf: Box<[u8; UART_READ_BUFFER_SIZE]>,
}

impl<R: AsyncRead + Unpin> UartReader<R> {
    /// Creates a new [`UartReader`] wrapping `client` with an empty initial buffer.
    pub fn new(client: R) -> Self {
        Self::with_initial_data(client, Vec::new())
    }

    /// Creates a new [`UartReader`] seeded with `initial_data` remaining from a prior handshake.
    pub fn with_initial_data(client: R, initial_data: Vec<u8>) -> Self {
        let mut parser = FrameParser::new();
        if !initial_data.is_empty() {
            parser.feed(&initial_data);
        }
        Self { client, parser, buf: Box::new([0u8; UART_READ_BUFFER_SIZE]) }
    }

    async fn drain_available(&mut self) {
        poll_fn(|cx| {
            loop {
                let mut read_buf = tokio::io::ReadBuf::new(&mut self.buf[..]);
                match Pin::new(&mut self.client).poll_read(cx, &mut read_buf) {
                    Poll::Ready(Ok(())) if !read_buf.filled().is_empty() => {
                        let n = read_buf.filled().len();
                        self.parser.feed(&self.buf[..n]);
                    }
                    _ => return Poll::Ready(()),
                }
            }
        })
        .await;
    }

    /// Reads from the underlying stream until a complete, checksum-verified [`Frame`] is decoded.
    pub async fn next_frame(
        &mut self,
        metrics: &Arc<Mutex<DaemonMetrics>>,
    ) -> Result<Frame, ReaderError> {
        loop {
            self.drain_available().await;
            let frame = self.parser.next_frame();
            metrics.lock().unwrap().checksum_errors = self.parser.checksum_errors();
            if let Some(frame) = frame {
                return Ok(frame);
            }
            let n = self.client.read(&mut self.buf[..]).await.map_err(ReaderError::Read)?;
            if n == 0 {
                return Err(ReaderError::Eof);
            }
            metrics.lock().unwrap().last_read_timestamp_ms = current_epoch_ms();
            self.parser.feed(&self.buf[..n]);
        }
    }
}

fn dispatch_ordered_event(
    session_id: u32,
    channel_id: u16,
    seq: u8,
    event_name: &'static str,
    event: UartEvent,
    receiver: &mut ResendReceiver,
    ack_tracker: &AckTracker,
    serial_tx: &mut mpsc::Sender<UartEvent>,
) -> Result<(), ReceiverTaskError> {
    // Go-Back-N Data Glitch & Loss Recovery:
    // If a UART wire glitch corrupts a frame (discarded by `FrameParser` via Fletcher-16 checksum)
    // or drops bytes, subsequent frames in the remote sender's window arrive with `seq != expected_seq`
    // (`FrameStatus::OutOfOrder`). We discard out-of-order/duplicate frames without advancing
    // `ResendReceiver` and immediately re-assert `ack_tracker.set_ack(rx_session_id, ack_seq)` for
    // the highest contiguous sequence number received prior to the glitch. This ensures the remote
    // `ResendSender` advances its window up to the last good frame and Go-Back-N retransmits
    // starting at `expected_seq`.
    //
    // Non-blocking `try_send` is critical here for two reasons:
    // 1. Deadlock Prevention: `receiver_task` must never suspend while forwarding `DATA` or
    //    `CLOSE` events to `coordinator_task`, so it remains free to read incoming `FrameType::Ack`
    //    and `FrameType::Reset` control frames from the UART even when `coordinator_task` is
    //    backpressured on a slow client socket.
    // 2. End-to-End Go-Back-N Backpressure: When `serial_tx` is full (`e.is_full()`), we drop the
    //    frame *without* calling `receiver.advance_in_order(seq)` and re-advertise the last
    //    contiguous `current_ack_seq()`. The remote `ResendSender` sees that its window has stopped
    //    advancing, pauses new transmissions once its window fills, and cleanly retransmits from
    //    `expected_seq` once `coordinator_task` drains `serial_rx`.
    let ack_seq = match receiver.inspect(seq) {
        FrameStatus::InOrder { .. } => match serial_tx.try_send(event) {
            Ok(()) => Some(receiver.advance_in_order(seq).map_err(ReceiverTaskError::AdvanceSeq)?),
            Err(e) if e.is_full() => {
                log::warn!(
                    "Coordinator queue full for channel {}; dropping in-order {} seq={}",
                    channel_id,
                    event_name,
                    seq
                );
                receiver.current_ack_seq()
            }
            Err(e) => {
                return Err(ReceiverTaskError::CoordinatorClosed(event_name, format!("{e:?}")));
            }
        },
        FrameStatus::OutOfOrder { expected_seq, ack_seq, .. } => {
            log::trace!(
                "Duplicate/Out-of-order {} frame, seq={}, expected={}",
                event_name,
                seq,
                expected_seq
            );
            ack_seq
        }
    };
    if let Some(seq) = ack_seq {
        ack_tracker.set_ack(session_id, seq);
    }
    Ok(())
}

fn process_received_frame(
    frame: Frame,
    receiver: &mut ResendReceiver,
    ack_tracker: &AckTracker,
    serial_tx: &mut mpsc::Sender<UartEvent>,
    incoming_event_tx: &mut mpsc::Sender<u8>,
) -> Result<(), ReceiverTaskError> {
    match frame.frame_type {
        FrameType::Data => {
            log::trace!(
                "Received DATA frame, seq={}, channel={}, len={}",
                frame.seq,
                frame.channel_id,
                frame.payload.len()
            );
            let ev = UartEvent::UartData { channel_id: frame.channel_id, data: frame.payload };
            dispatch_ordered_event(
                frame.session_id,
                frame.channel_id,
                frame.seq,
                "DATA",
                ev,
                receiver,
                ack_tracker,
                serial_tx,
            )?;
        }
        FrameType::Ack => {
            log::trace!("Received ACK frame, seq={}", frame.seq);
            // Use non-blocking `try_send` so `process_received_frame` is purely synchronous and
            // can never suspend `receiver_task`. Because FPL ACKs are cumulative, if `incoming_event_tx`
            // is momentarily full, any subsequent ACK supersedes earlier ones.
            if let Err(e) = incoming_event_tx.try_send(frame.seq) {
                if e.is_full() {
                    log::warn!("Sender ACK queue full, dropping cumulative ACK seq={}", frame.seq);
                } else {
                    return Err(ReceiverTaskError::EnqueueAck(e.into_send_error()));
                }
            }
        }
        FrameType::Close => {
            log::trace!("Received CLOSE frame, seq={}, channel={}", frame.seq, frame.channel_id);
            let ev = UartEvent::UartClose { channel_id: frame.channel_id };
            dispatch_ordered_event(
                frame.session_id,
                frame.channel_id,
                frame.seq,
                "CLOSE",
                ev,
                receiver,
                ack_tracker,
                serial_tx,
            )?;
        }
        _ => log::warn!("Unknown frame type: {}", u8::from(frame.frame_type)),
    }
    Ok(())
}

async fn run_receiver_loop<R: AsyncRead + Unpin>(
    reader: &mut UartReader<R>,
    serial_tx: &mut mpsc::Sender<UartEvent>,
    ack_tracker: &AckTracker,
    incoming_event_tx: &mut mpsc::Sender<u8>,
    session_id: u32,
    metrics: &Arc<Mutex<DaemonMetrics>>,
) -> Result<(), ReceiverTaskError> {
    let mut receiver = ResendReceiver::new();
    loop {
        let frame = reader.next_frame(metrics).await?;
        if frame.frame_type == FrameType::Reset
            || (frame.frame_type == FrameType::Close && frame.channel_id == CONTROL_CHANNEL_ID)
        {
            log::info!("Received RESET frame from target. Restarting UART driver...");
            return Err(ReceiverTaskError::TargetRequestedReset);
        }
        if frame.session_id != session_id {
            log::warn!(
                "Received frame with mismatched session ID (got {}, expected {}), discarding",
                frame.session_id,
                session_id
            );
            continue;
        }
        process_received_frame(frame, &mut receiver, ack_tracker, serial_tx, incoming_event_tx)?;
        // Yield to the executor to allow other tasks on the single-threaded runtime (such as
        // coordinator_task consuming serial_tx and writer_task transmitting cumulative ACKs)
        // to make progress when a single I/O read chunk delivers multiple parsed frames
        // without suspension.
        fuchsia_async::yield_now().await;
    }
}

/// Demultiplexes incoming UART frames using [`uart_fpl::ResendReceiver`] and generates cumulative ACKs.
pub async fn receiver_task<R: AsyncRead + Unpin>(
    mut reader: UartReader<R>,
    mut serial_tx: mpsc::Sender<UartEvent>,
    ack_tracker: AckTracker,
    mut incoming_event_tx: mpsc::Sender<u8>,
    session_id: u32,
    metrics: Arc<Mutex<DaemonMetrics>>,
) -> Result<(), TaskError> {
    run_receiver_loop(
        &mut reader,
        &mut serial_tx,
        &ack_tracker,
        &mut incoming_event_tx,
        session_id,
        &metrics,
    )
    .await
    .map_err(TaskError::Receiver)
}

fn handle_sender_ack(
    ack_seq: u8,
    sender: &mut ResendSender,
    active_timer: &mut Option<fuchsia_async::Timer>,
    metrics: &Arc<Mutex<DaemonMetrics>>,
) {
    if let AckOutcome::Advanced { rtt, all_acked, .. } = sender.handle_ack(ack_seq) {
        if let Some(rtt) = rtt {
            let mut m = metrics.lock().unwrap();
            m.estimated_rtt_ms = rtt.as_millis() as u32;
        }
        if all_acked {
            *active_timer = None;
        } else {
            *active_timer = Some(fuchsia_async::Timer::new(DEFAULT_RETRANSMISSION_TIMEOUT));
        }
    }
}

async fn send_data_frame(
    frame: Vec<u8>,
    sender: &mut ResendSender,
    active_timer: &mut Option<fuchsia_async::Timer>,
    data_tx: &mut mpsc::Sender<Vec<u8>>,
    incoming_event_rx: &mut mpsc::Receiver<u8>,
    metrics: &Arc<Mutex<DaemonMetrics>>,
) -> Result<(), SenderTaskError> {
    // Bidirectional Deadlock Prevention:
    // When the UART writer path is slow or rate-limited (`data_tx` is full), `sender_task`
    // suspends while pushing outgoing or retransmitted frames into `data_tx`. We must continue
    // polling `incoming_event_rx` via `select_biased!` while `data_tx.send(frame)` is pending so
    // incoming cumulative ACKs are drained immediately, advancing the Go-Back-N window and
    // resetting/clearing `active_timer` rather than stalling behind `data_tx`.
    let mut send_fut = std::pin::pin!(data_tx.send(frame).fuse());
    loop {
        futures::select_biased! {
            ack_event = incoming_event_rx.next().fuse() => match ack_event {
                Some(seq) => handle_sender_ack(seq, sender, active_timer, metrics),
                None => return Err(SenderTaskError::AckChannelClosed),
            },
            res = send_fut => {
                return res.map_err(|_| SenderTaskError::WriterClosed);
            }
        }
    }
}

async fn handle_sender_timeout(
    sender: &mut ResendSender,
    active_timer: &mut Option<fuchsia_async::Timer>,
    data_tx: &mut mpsc::Sender<Vec<u8>>,
    incoming_event_rx: &mut mpsc::Receiver<u8>,
    metrics: &Arc<Mutex<DaemonMetrics>>,
) -> Result<(), SenderTaskError> {
    let retransmit_frames =
        sender.handle_timeout().map_err(SenderTaskError::RetransmissionLimit)?;
    {
        let mut m = metrics.lock().unwrap();
        m.retransmissions += 1;
    }
    log::trace!(
        "Go-Back-N timeout! Retransmitting window base={}, next_seq={}, attempts={}",
        sender.base(),
        sender.next_seq(),
        sender.retransmission_attempts()
    );

    for frame in retransmit_frames {
        // Because `send_data_frame` continues processing incoming ACKs while waiting on `data_tx`,
        // a cumulative ACK arriving mid-burst can acknowledge the entire in-flight window. Stop
        // retransmitting immediately if `sender.is_idle()` becomes true.
        if sender.is_idle() {
            break;
        }
        send_data_frame(frame, sender, active_timer, data_tx, incoming_event_rx, metrics).await?;
    }

    if !sender.is_idle() {
        *active_timer = Some(fuchsia_async::Timer::new(DEFAULT_RETRANSMISSION_TIMEOUT));
    }
    Ok(())
}

async fn send_next_message(
    msg: SenderMessage,
    sender: &mut ResendSender,
    active_timer: &mut Option<fuchsia_async::Timer>,
    data_tx: &mut mpsc::Sender<Vec<u8>>,
    incoming_event_rx: &mut mpsc::Receiver<u8>,
    sid: u32,
    metrics: &Arc<Mutex<DaemonMetrics>>,
) -> Result<(), SenderTaskError> {
    let (frame_type, channel_id, payload) = match msg {
        SenderMessage::Data { channel_id, payload } => (FrameType::Data, channel_id, payload),
        SenderMessage::Close { channel_id } => (FrameType::Close, channel_id, Vec::new()),
    };

    let (_seq, frame) = sender
        .enqueue_frame(sid, channel_id, frame_type, &payload)
        .map_err(SenderTaskError::EncodeFrame)?;

    // Arm the retransmission timer before calling `send_data_frame` so that if an ACK for this
    // frame arrives while `send_data_frame` is waiting for `data_tx` to flush, `handle_sender_ack`
    // will cleanly clear `active_timer` back to `None`.
    if sender.in_flight() == 1 {
        *active_timer = Some(fuchsia_async::Timer::new(DEFAULT_RETRANSMISSION_TIMEOUT));
    }

    send_data_frame(frame, sender, active_timer, data_tx, incoming_event_rx, metrics).await?;
    Ok(())
}

async fn wait_active_timer(active_timer: &mut Option<fuchsia_async::Timer>) {
    if let Some(timer) = active_timer {
        timer.await;
    } else {
        futures::future::pending::<()>().await;
    }
}

async fn run_sender_loop(
    mut sender_rx: mpsc::Receiver<SenderMessage>,
    mut data_tx: mpsc::Sender<Vec<u8>>,
    mut incoming_event_rx: mpsc::Receiver<u8>,
    session_id: u32,
    metrics: &Arc<Mutex<DaemonMetrics>>,
) -> Result<(), SenderTaskError> {
    let mut sender = ResendSender::default();
    let mut active_timer: Option<fuchsia_async::Timer> = None;
    let mut sender_rx_closed = false;
    loop {
        let mut msg_rx_fut = std::pin::pin!(if sender.can_send() && !sender_rx_closed {
            futures::future::Either::Left(sender_rx.next())
        } else {
            futures::future::Either::Right(futures::future::pending())
        });
        futures::select_biased! {
            ack_event = incoming_event_rx.next().fuse() => match ack_event {
                Some(seq) => handle_sender_ack(seq, &mut sender, &mut active_timer, metrics),
                None => return Err(SenderTaskError::AckChannelClosed),
            },
            _ = wait_active_timer(&mut active_timer).fuse() => {
                handle_sender_timeout(
                    &mut sender,
                    &mut active_timer,
                    &mut data_tx,
                    &mut incoming_event_rx,
                    metrics,
                )
                .await?;
            }
            msg = msg_rx_fut => match msg {
                Some(m) => {
                    send_next_message(
                        m,
                        &mut sender,
                        &mut active_timer,
                        &mut data_tx,
                        &mut incoming_event_rx,
                        session_id,
                        metrics,
                    )
                    .await?
                }
                None => sender_rx_closed = true,
            },
        }
        if sender_rx_closed && sender.is_idle() {
            break;
        }
    }
    Ok(())
}

/// Packetizes outgoing messages and manages sliding-window Go-Back-N retransmission via [`uart_fpl::ResendSender`].
pub async fn sender_task(
    sender_rx: mpsc::Receiver<SenderMessage>,
    data_tx: mpsc::Sender<Vec<u8>>,
    incoming_event_rx: mpsc::Receiver<u8>,
    session_id: u32,
    metrics: Arc<Mutex<DaemonMetrics>>,
) -> Result<(), TaskError> {
    run_sender_loop(sender_rx, data_tx, incoming_event_rx, session_id, &metrics)
        .await
        .map_err(TaskError::Sender)
}

/// Host-side driver coordinator managing daemon background execution and hardware state.
///
/// Supervises client connections, monitors metadata file deletion and peer exit
/// for graceful termination, and updates connection metadata states.
pub struct HostDriver;

impl HostDriver {
    /// Updates connection metadata status and clears nodename/serial when disconnecting.
    pub fn update_status(meta_path: &Option<PathBuf>, status: ConnectionStatus) {
        let Some(path) = meta_path else { return };
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("Failed to read metadata at {}: {:?}", path.display(), e);
                return;
            }
        };
        let mut meta = match serde_json::from_str::<ConnectionMetadata>(&content) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("Failed to parse metadata at {}: {:?}", path.display(), e);
                return;
            }
        };
        meta.status = status;
        if meta.status != ConnectionStatus::Connected {
            meta.nodename = None;
            meta.serial = None;
        }
        if let Ok(new_content) = serde_json::to_string(&meta) {
            let temp_path = path.with_extension(format!("{}.tmp", meta.pid));
            if std::fs::write(&temp_path, new_content.as_bytes()).is_ok() {
                let _ = std::fs::rename(&temp_path, path);
            }
        }
    }

    /// Spawns a background task to watch for deletion of the connection metadata file,
    /// signaling `shutdown_tx` to initiate graceful termination when deleted.
    pub fn watch_metadata_deletion(
        meta_path: &Option<PathBuf>,
        shutdown_tx: futures::channel::oneshot::Sender<()>,
    ) -> Option<fuchsia_async::Task<()>> {
        let path = meta_path.as_ref()?.clone();
        Some(fuchsia_async::Task::local(async move {
            loop {
                fuchsia_async::Timer::new(METADATA_CHECK_INTERVAL).await;
                if !path.exists() {
                    log::info!(
                        "Metadata file {} was deleted. Triggering shutdown.",
                        path.display()
                    );
                    let _ = shutdown_tx.send(());
                    break;
                }
            }
        }))
    }

    /// Deletes local Unix domain socket and metadata files upon daemon termination.
    pub fn cleanup_driver_files(
        local_addr: Option<tokio::net::unix::SocketAddr>,
        meta_path: Option<&PathBuf>,
    ) {
        if let Some(path) = local_addr.as_ref().and_then(|a| a.as_pathname()) {
            if let Err(e) = std::fs::remove_file(path) {
                log::warn!("Failed to remove socket file at {}: {:?}", path.display(), e);
            }
        }
        if let Some(path) = meta_path {
            if let Err(e) = std::fs::remove_file(path) {
                log::warn!("Failed to remove metadata file at {}: {:?}", path.display(), e);
            }
        }
    }

    /// Inspects connection errors to determine if reconnection should be aborted.
    pub fn check_fatal_connect_error(
        e: &ConnectionError,
        target_path: &str,
        no_retry: bool,
        is_socket: bool,
        associated_peer_pid: Option<u32>,
    ) -> bool {
        if no_retry {
            log::error!("Connection failed and no-retry is set. Exiting driver: {e}");
            return true;
        }
        if let Some(pid) = associated_peer_pid {
            if !ffx_tool_uart::is_running(pid) {
                log::info!("Associated peer (PID {pid}) has exited. Exiting driver.");
                return true;
            }
        }
        if matches!(e, ConnectionError::PathNotFound { .. }) {
            if is_socket {
                log::info!(
                    "Virtual socket target path {target_path} does not exist. Exiting driver."
                );
                return true;
            }
            log::warn!("Target UART port {target_path} not found. Waiting for device...");
        }
        false
    }
}

#[cfg(test)]
fn make_test_metadata(target: &str, status: ConnectionStatus) -> ConnectionMetadata {
    ConnectionMetadata {
        pid: std::process::id(),
        target: target.to_string(),
        status,
        id: Some("testid".to_string()),
        baud: NonZeroU32::new(115200),
        protocol: UartProtocol::ResendSP,
        log_level: None,
        nodename: None,
        serial: None,
    }
}

#[cfg(test)]
fn write_test_metadata(
    path: &std::path::Path,
    target: &str,
    pid: u32,
    status: ConnectionStatus,
) -> ConnectionMetadata {
    let mut meta = make_test_metadata(target, status);
    meta.pid = pid;
    std::fs::write(path, serde_json::to_string(&meta).unwrap()).unwrap();
    meta
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tokio::io::ReadBuf;

    #[fuchsia::test]
    async fn test_uart_reader_drain_and_frame() {
        let (mut client, server) = tokio::io::duplex(1024);
        let mut reader = UartReader::new(server);
        let metrics = Arc::new(Mutex::new(DaemonMetrics::default()));

        let frame_bytes = encode_frame(123, 1, 0, FrameType::Data, b"hello uart").unwrap();
        client.write_all(&frame_bytes).await.unwrap();

        let frame = reader.next_frame(&metrics).await.unwrap();
        assert_eq!(frame.session_id, 123);
        assert_eq!(frame.channel_id, 1);
        assert_eq!(frame.seq, 0);
        assert_eq!(frame.payload, b"hello uart");
    }

    #[fuchsia::test]
    async fn test_remove_and_bind_socket() {
        let temp = tempfile::tempdir().unwrap();
        let sock_path = temp.path().join("test.sock");

        let listener = remove_and_bind_socket(sock_path.clone(), false).await.unwrap();
        drop(listener);

        // Bind again to verify stale socket removal
        let listener2 = remove_and_bind_socket(sock_path, false).await.unwrap();
        drop(listener2);
    }

    #[fuchsia::test]
    async fn test_sender_task_enqueue_and_ack() {
        let (mut sender_tx, sender_rx) = mpsc::channel(16);
        let (data_tx, mut data_rx) = mpsc::channel(16);
        let (mut incoming_event_tx, incoming_event_rx) = mpsc::channel(16);
        let session_id = 100;
        let metrics = Arc::new(Mutex::new(DaemonMetrics::default()));

        let task = fuchsia_async::Task::local(sender_task(
            sender_rx,
            data_tx,
            incoming_event_rx,
            session_id,
            metrics,
        ));

        // Send a data message
        sender_tx
            .send(SenderMessage::Data { channel_id: 1, payload: b"ping".to_vec() })
            .await
            .unwrap();

        // Check that data_rx receives an encoded frame
        let frame_bytes = data_rx.next().await.unwrap();
        let mut parser = FrameParser::new();
        parser.feed(&frame_bytes);
        let frame = parser.next_frame().unwrap();
        assert_eq!(frame.channel_id, 1);
        assert_eq!(frame.payload, b"ping");

        // Send ACK for seq
        incoming_event_tx.send(frame.seq).await.unwrap();

        // Close sender_tx while keeping incoming_event_tx alive
        drop(sender_tx);

        let res = task.await;
        assert!(res.is_ok(), "sender_task failed: {:?}", res);
    }

    #[fuchsia::test]
    async fn test_coordinator_task_client_data_and_close() {
        let (mut client_tx_out, client_rx) = mpsc::channel(16);
        let (_serial_tx, serial_rx) = mpsc::channel(16);
        let (client_tx, _client_rx_in) = mpsc::channel(16);
        let (sender_tx_ch, mut sender_rx_ch) = mpsc::channel(16);
        let metrics = Arc::new(Mutex::new(DaemonMetrics::default()));

        let coord_task = fuchsia_async::Task::local(coordinator_task(
            client_rx,
            serial_rx,
            client_tx,
            Some(sender_tx_ch),
            metrics,
        ));

        // Send channel data
        client_tx_out
            .send(ClientEvent::ChannelData { channel_id: 1, data: b"hello".to_vec() })
            .await
            .unwrap();

        // Expect sender_rx_ch receives SenderMessage::Data
        let msg = sender_rx_ch.next().await.unwrap();
        match msg {
            SenderMessage::Data { channel_id, payload } => {
                assert_eq!(channel_id, 1);
                assert_eq!(payload, b"hello");
            }
            _ => panic!("Expected SenderMessage::Data"),
        }

        // Close client_tx_out to terminate coordinator
        drop(client_tx_out);
        let res = coord_task.await;
        assert!(res.is_ok());
    }

    #[fuchsia::test]
    async fn test_sender_task_processes_acks_while_data_tx_blocked() {
        let (mut sender_tx, sender_rx) = mpsc::channel(16);
        // Create data_tx with capacity 1 (2 slots total with 1 sender) so f0 completes
        // poll_flush and f1 suspends inside send_data_frame with both seq 0 and seq 1 in flight.
        let (data_tx, mut data_rx) = mpsc::channel(1);
        let (mut incoming_event_tx, incoming_event_rx) = mpsc::channel(1);
        let session_id = 100;
        let metrics = Arc::new(Mutex::new(DaemonMetrics::default()));

        let task = fuchsia_async::Task::local(sender_task(
            sender_rx,
            data_tx,
            incoming_event_rx,
            session_id,
            metrics,
        ));

        // 1st frame occupies slot 1 in data_tx; 2nd frame fills slot 2 and suspends in poll_flush
        sender_tx
            .send(SenderMessage::Data { channel_id: 1, payload: b"f0".to_vec() })
            .await
            .unwrap();
        sender_tx
            .send(SenderMessage::Data { channel_id: 1, payload: b"f1".to_vec() })
            .await
            .unwrap();
        // Yield so sender_task dequeues f0 and f1 and suspends inside send_data_frame(f1)
        fuchsia_async::yield_now().await;
        fuchsia_async::yield_now().await;

        // While sender_task is blocked waiting for data_rx to drain, send ACKs for seq 0 and 1
        // and yield so sender_task processes them inside send_data_frame while data_tx is full.
        incoming_event_tx.send(0).await.unwrap();
        incoming_event_tx.send(1).await.unwrap();
        fuchsia_async::yield_now().await;

        // Now drain data_rx and close sender_tx
        let _ = data_rx.next().await.unwrap();
        let _ = data_rx.next().await.unwrap();
        drop(sender_tx);

        assert!(task.await.is_ok());
    }

    #[fuchsia::test]
    async fn test_coordinator_congested_client_backpressure_and_disconnect() {
        let (mut client_tx_out, client_rx) = mpsc::channel(1);
        let (mut serial_tx, serial_rx) = mpsc::channel(16);
        let (sender_tx_ch, mut sender_rx_ch) = mpsc::channel(16);
        let metrics = Arc::new(Mutex::new(DaemonMetrics::default()));

        let coord_task = fuchsia_async::Task::local(coordinator_task(
            client_rx,
            serial_rx,
            client_tx_out.clone(),
            Some(sender_tx_ch),
            metrics,
        ));

        let (stalled_client, stalled_server) = UnixStream::pair().unwrap();
        let (mut healthy_client, healthy_server) = UnixStream::pair().unwrap();

        client_tx_out
            .send(ClientEvent::NewChannel { channel_id: 1, stream: stalled_server })
            .await
            .unwrap();
        client_tx_out
            .send(ClientEvent::NewChannel { channel_id: 2, stream: healthy_server })
            .await
            .unwrap();

        // Flood channel 1 with large payloads until `state.writer_tx` blocks `handle_uart_event`
        let big_chunk = vec![b'x'; 65536];
        for _ in 0..(CLIENT_WRITER_CHANNEL_CAPACITY + 64) {
            if serial_tx
                .try_send(UartEvent::UartData { channel_id: 1, data: big_chunk.clone() })
                .is_err()
            {
                break;
            }
            fuchsia_async::yield_now().await;
        }

        // Disconnect `stalled_client` while `coordinator_task` is backpressured on `writer_tx`.
        // Because `client_writer_task` drops `receiver` and `writer` before awaiting
        // `coordinator_tx.send(ChannelClose)`, `state.writer_tx.send` immediately unblocks.
        drop(stalled_client);

        // Verify channel 2 receives data cleanly after the stalled client disconnects
        serial_tx
            .send(UartEvent::UartData { channel_id: 2, data: b"healthy-ok".to_vec() })
            .await
            .unwrap();

        let mut buf = [0u8; 10];
        healthy_client.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"healthy-ok");

        // Verify coordinator emitted a Close message for the disconnected channel 1
        let msg = sender_rx_ch.next().await.unwrap();
        assert!(matches!(msg, SenderMessage::Close { channel_id: 1 }));

        drop(healthy_client);
        drop(client_tx_out);
        drop(serial_tx);
        assert!(coord_task.await.is_ok());
    }

    #[fuchsia::test]
    async fn test_writer_task_batches_and_acks() {
        let (client, mut server) = tokio::io::duplex(1024);
        let ack_tracker = AckTracker::new();
        let (mut data_tx, data_rx) = mpsc::channel(16);
        let metrics = Arc::new(Mutex::new(DaemonMetrics::default()));
        let task = fuchsia_async::Task::local(writer_task(
            client,
            ack_tracker.clone(),
            data_rx,
            metrics,
            None,
        ));

        ack_tracker.set_ack(42, 7);
        let mut buf = [0u8; 64];
        let n = server.read(&mut buf).await.unwrap();
        let mut parser = FrameParser::new();
        parser.feed(&buf[..n]);
        let frame = parser.next_frame().unwrap();
        assert_eq!((frame.session_id, frame.seq, frame.frame_type), (42, 7, FrameType::Ack));

        let data_frame = encode_frame(42, 1, 0, FrameType::Data, b"out").unwrap();
        data_tx.send(data_frame).await.unwrap();
        drop(data_tx);

        let n2 = server.read(&mut buf).await.unwrap();
        parser.feed(&buf[..n2]);
        let frame2 = parser.next_frame().unwrap();
        assert_eq!((frame2.channel_id, frame2.payload.as_slice()), (1, &b"out"[..]));
        assert!(task.await.is_ok());
    }

    #[fuchsia::test]
    async fn test_writer_task_rate_limiting() {
        let (client, mut server) = tokio::io::duplex(1024);
        let ack_tracker = AckTracker::new();
        let (mut data_tx, data_rx) = mpsc::channel(16);
        let metrics = Arc::new(Mutex::new(DaemonMetrics::default()));
        let task = fuchsia_async::Task::local(writer_task(
            client,
            ack_tracker,
            data_rx,
            metrics,
            Some(100_000),
        ));

        let data_frame = encode_frame(42, 1, 0, FrameType::Data, b"rate-limited").unwrap();
        data_tx.send(data_frame).await.unwrap();
        drop(data_tx);

        let mut buf = [0u8; 64];
        let n = server.read(&mut buf).await.unwrap();
        let mut parser = FrameParser::new();
        parser.feed(&buf[..n]);
        let frame = parser.next_frame().unwrap();
        assert_eq!((frame.channel_id, frame.payload.as_slice()), (1, &b"rate-limited"[..]));
        assert!(task.await.is_ok());
    }

    #[fuchsia::test]
    async fn test_receiver_task_in_order_and_ack() {
        let (mut client, server) = tokio::io::duplex(1024);
        let reader = UartReader::new(server);
        let (serial_tx, mut serial_rx) = mpsc::channel(16);
        let ack_tracker = AckTracker::new();
        let (incoming_event_tx, _incoming_event_rx) = mpsc::channel(16);
        let metrics = Arc::new(Mutex::new(DaemonMetrics::default()));

        let task = fuchsia_async::Task::local(receiver_task(
            reader,
            serial_tx,
            ack_tracker.clone(),
            incoming_event_tx,
            99,
            metrics,
        ));

        let data_frame = encode_frame(99, 2, 0, FrameType::Data, b"rx-payload").unwrap();
        client.write_all(&data_frame).await.unwrap();

        let event = serial_rx.next().await.unwrap();
        match event {
            UartEvent::UartData { channel_id, data } => {
                assert_eq!(channel_id, 2);
                assert_eq!(data, b"rx-payload");
            }
            _ => panic!("Expected UartEvent::UartData"),
        }
        assert_eq!(ack_tracker.take_ack(), Some((99, 0)));

        drop(client);
        let _ = task.await;
    }

    #[fuchsia::test]
    async fn test_receiver_task_reset_frame_error() {
        let (mut client, server) = tokio::io::duplex(1024);
        let reader = UartReader::new(server);
        let (serial_tx, _serial_rx) = mpsc::channel(16);
        let ack_tracker = AckTracker::new();
        let (incoming_event_tx, _incoming_event_rx) = mpsc::channel(16);
        let metrics = Arc::new(Mutex::new(DaemonMetrics::default()));

        let task = fuchsia_async::Task::local(receiver_task(
            reader,
            serial_tx,
            ack_tracker,
            incoming_event_tx,
            99,
            metrics,
        ));

        let reset_frame = encode_frame(99, CONTROL_CHANNEL_ID, 0, FrameType::Reset, &[]).unwrap();
        client.write_all(&reset_frame).await.unwrap();

        let res = task.await;
        assert!(matches!(res, Err(TaskError::Receiver(ReceiverTaskError::TargetRequestedReset))));
    }

    #[fuchsia::test]
    async fn test_sender_task_ack_channel_closed() {
        let (_sender_tx, sender_rx) = mpsc::channel(16);
        let (data_tx, _data_rx) = mpsc::channel(16);
        let (incoming_event_tx, incoming_event_rx) = mpsc::channel(16);
        let session_id = 100;
        let metrics = Arc::new(Mutex::new(DaemonMetrics::default()));

        let task = fuchsia_async::Task::local(sender_task(
            sender_rx,
            data_tx,
            incoming_event_rx,
            session_id,
            metrics,
        ));

        // Drop incoming_event_tx so the ACK channel closes
        drop(incoming_event_tx);

        let res = task.await;
        assert!(matches!(res, Err(TaskError::Sender(SenderTaskError::AckChannelClosed))));
    }

    #[fuchsia::test]
    async fn test_uart_reader_eof() {
        let (client, server) = tokio::io::duplex(1024);
        drop(client);
        let mut reader = UartReader::new(server);
        let metrics = Arc::new(Mutex::new(DaemonMetrics::default()));

        let res = reader.next_frame(&metrics).await;
        assert!(matches!(res, Err(ReaderError::Eof)));
    }

    struct MockReader {
        data: Vec<u8>,
        read_ptr: usize,
    }

    impl MockReader {
        fn new(data: Vec<u8>) -> Self {
            Self { data, read_ptr: 0 }
        }
    }

    impl tokio::io::AsyncRead for MockReader {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            if self.read_ptr >= self.data.len() {
                return Poll::Ready(Ok(()));
            }
            let amt = std::cmp::min(buf.remaining(), self.data.len() - self.read_ptr);
            buf.put_slice(&self.data[self.read_ptr..self.read_ptr + amt]);
            self.read_ptr += amt;
            Poll::Ready(Ok(()))
        }
    }

    #[fuchsia::test]
    async fn test_watch_metadata_deletion_unit() {
        let temp = tempfile::tempdir().unwrap();
        let meta_path = temp.path().join("meta.json");
        write_test_metadata(
            &meta_path,
            "test_target",
            std::process::id(),
            ConnectionStatus::Connected,
        );

        let (shutdown_tx, shutdown_rx) = futures::channel::oneshot::channel();
        let _watcher = HostDriver::watch_metadata_deletion(&Some(meta_path.clone()), shutdown_tx);

        std::fs::remove_file(&meta_path).unwrap();
        let timeout_res = tokio::time::timeout(Duration::from_secs(3), shutdown_rx).await;
        assert!(timeout_res.is_ok(), "Shutdown signal not received within timeout");
    }

    #[fuchsia::test]
    async fn test_host_receiver_task_session_change_discard() {
        let (s1, s2, cid) = (12345u32, 67890u32, 42u16);
        let mut data = encode_frame(s2, cid, 0, FrameType::Data, b"discard").unwrap();
        data.extend_from_slice(&encode_frame(s1, cid, 0, FrameType::Data, b"accept").unwrap());

        let (serial_tx, mut serial_rx) = mpsc::channel(10);
        let ack_tracker = AckTracker::new();
        let (incoming_event_tx, _incoming_event_rx) = mpsc::channel(10);
        let metrics = Arc::new(Mutex::new(DaemonMetrics::default()));

        let rx_handle = fuchsia_async::Task::local(receiver_task(
            UartReader::new(MockReader::new(data)),
            serial_tx,
            ack_tracker.clone(),
            incoming_event_tx,
            s1,
            metrics,
        ));

        let event = serial_rx.next().await.unwrap();
        assert_eq!(event, UartEvent::UartData { channel_id: cid, data: b"accept".to_vec() });
        assert_eq!(ack_tracker.take_ack(), Some((s1, 0)));
        drop(rx_handle);
    }

    #[fuchsia::test]
    async fn test_host_receiver_task_fletcher_collision_noise() {
        let (s_cur, s_noise) = (0xABCDEFu32, 0x99999999u32);
        let mut data = encode_frame(s_noise, 1, 0, FrameType::Data, b"noisebytes").unwrap();
        data.extend_from_slice(&encode_frame(s_cur, 1, 0, FrameType::Data, b"validbytes").unwrap());

        let (serial_tx, mut serial_rx) = mpsc::channel(10);
        let ack_tracker = AckTracker::new();
        let (incoming_event_tx, _incoming_event_rx) = mpsc::channel(10);
        let metrics = Arc::new(Mutex::new(DaemonMetrics::default()));

        let rx_handle = fuchsia_async::Task::local(receiver_task(
            UartReader::new(MockReader::new(data)),
            serial_tx,
            ack_tracker,
            incoming_event_tx,
            s_cur,
            metrics,
        ));

        let event = serial_rx.next().await.unwrap();
        assert_eq!(event, UartEvent::UartData { channel_id: 1, data: b"validbytes".to_vec() });
        drop(rx_handle);
    }

    #[fuchsia::test]
    async fn test_uart_reader_with_initial_data() {
        let session_id = 12345u32;
        let payload = b"unconsumed_hello";
        let frame = encode_frame(session_id, 1, 0, FrameType::Data, payload).unwrap();

        let mut reader = UartReader::with_initial_data(MockReader::new(Vec::new()), frame);
        let metrics = Arc::new(Mutex::new(DaemonMetrics::default()));

        let parsed = reader.next_frame(&metrics).await.unwrap();
        assert_eq!(parsed.session_id, session_id);
        assert_eq!(parsed.channel_id, 1);
        assert_eq!(parsed.payload, payload);
    }
}
