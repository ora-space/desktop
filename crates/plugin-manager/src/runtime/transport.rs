use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use ora_plugin_protocol::{
    FrameDecoder, FrameError, HostRequestId, JsonRpcEnvelope, MAX_FRAME_BYTES, encode_json_frame,
    parse_json_rpc_frame,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot};

use crate::{IoFailure, ProtocolFailure, WriterFailureStage};

const READ_CHUNK_BYTES: usize = 64 * 1024;

/// Fixed writer queue and byte reserves; ordinary work cannot borrow lifecycle/safety capacity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriterQueueLimits {
    pub ordinary_frames: usize,
    pub ordinary_bytes: usize,
    pub transport_cancel_frames: usize,
    pub transport_cancel_bytes: usize,
    pub session_control_frames: usize,
    pub session_control_bytes: usize,
}

impl WriterQueueLimits {
    pub fn v1_defaults() -> Self {
        Self {
            ordinary_frames: 224,
            ordinary_bytes: 14 * 1024 * 1024,
            transport_cancel_frames: 16,
            transport_cancel_bytes: 1024 * 1024,
            session_control_frames: 16,
            session_control_bytes: 1024 * 1024,
        }
    }

    /// Rejects a profile that does not preserve the frozen 256-frame/16-MiB aggregate budget.
    pub fn validate(&self) -> Result<(), WriterQueueConfigError> {
        let frame_total =
            self.ordinary_frames + self.transport_cancel_frames + self.session_control_frames;
        let byte_total =
            self.ordinary_bytes + self.transport_cancel_bytes + self.session_control_bytes;
        if self.ordinary_frames == 0
            || self.ordinary_bytes == 0
            || self.transport_cancel_frames == 0
            || self.transport_cancel_bytes == 0
            || self.session_control_frames == 0
            || self.session_control_bytes == 0
            || frame_total != 256
            || byte_total != 16 * 1024 * 1024
        {
            return Err(WriterQueueConfigError::InvalidV1Budget);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WriterQueueConfigError {
    #[error("writer v1 budgets must total 256 frames/16 MiB with full-frame control reserves")]
    InvalidV1Budget,
}

/// Identifies one writer command without conflating it with a peer acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriterCommandOwner {
    Request(HostRequestId),
    TransportCancel(HostRequestId),
    SessionControl(SessionControlKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionControlKind {
    Initialize,
    Activate,
    Deactivate,
    Exit,
}

/// Non-borrowable writer lanes corresponding to the three configured reserves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriterLane {
    Ordinary,
    TransportCancel,
    SessionControl,
}

impl WriterLane {
    const fn failure_stage(self) -> WriterFailureStage {
        match self {
            Self::Ordinary => WriterFailureStage::Request,
            Self::TransportCancel => WriterFailureStage::TransportCancel,
            Self::SessionControl => WriterFailureStage::SessionControl,
        }
    }
}

/// A complete encoded frame whose queue-byte permit follows it through the writer.
#[derive(Debug)]
struct WriterCommand {
    generation: u64,
    owner: WriterCommandOwner,
    frame: Vec<u8>,
    lane: WriterLane,
    _byte_permit: OwnedSemaphorePermit,
}

/// The single final local-write fact returned by the writer for each accepted command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriterCompletion {
    FrameWritten {
        generation: u64,
        owner: WriterCommandOwner,
    },
    WriteFailed {
        generation: u64,
        owner: WriterCommandOwner,
        bytes_written: Option<usize>,
        stage: WriterFailureStage,
        failure: IoFailure,
    },
}

/// Cloneable producer set; the writer task remains the sole stdin owner.
#[derive(Debug, Clone)]
pub struct WriterQueues {
    ordinary: mpsc::Sender<WriterCommand>,
    transport_cancel: mpsc::Sender<WriterCommand>,
    session_control: mpsc::Sender<WriterCommand>,
    ordinary_bytes: Arc<Semaphore>,
    transport_cancel_bytes: Arc<Semaphore>,
    session_control_bytes: Arc<Semaphore>,
}

impl WriterQueues {
    /// Encodes and reserves one complete JSON frame within the caller's absolute enqueue budget.
    pub async fn enqueue(
        &self,
        generation: u64,
        owner: WriterCommandOwner,
        payload: &[u8],
        lane: WriterLane,
        timeout: Duration,
    ) -> Result<(), WriterEnqueueError> {
        let frame = encode_json_frame(payload, MAX_FRAME_BYTES).map_err(WriterEnqueueError::Frame)?;
        let permits = u32::try_from(frame.len()).map_err(|_| WriterEnqueueError::BudgetClosed)?;
        let (semaphore, sender) = match lane {
            WriterLane::Ordinary => (&self.ordinary_bytes, &self.ordinary),
            WriterLane::TransportCancel => (&self.transport_cancel_bytes, &self.transport_cancel),
            WriterLane::SessionControl => (&self.session_control_bytes, &self.session_control),
        };
        let permit =
            tokio::time::timeout(timeout, Arc::clone(semaphore).acquire_many_owned(permits))
                .await
                .map_err(|_| WriterEnqueueError::DeadlineExceeded)?
                .map_err(|_| WriterEnqueueError::BudgetClosed)?;
        let command = WriterCommand {
            generation,
            owner,
            frame,
            lane,
            _byte_permit: permit,
        };
        tokio::time::timeout(timeout, sender.send(command))
            .await
            .map_err(|_| WriterEnqueueError::DeadlineExceeded)?
            .map_err(|_| WriterEnqueueError::WriterClosed)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WriterEnqueueError {
    #[error(transparent)]
    Frame(FrameError),
    #[error("writer enqueue deadline exceeded")]
    DeadlineExceeded,
    #[error("writer byte budget is closed")]
    BudgetClosed,
    #[error("writer task is closed")]
    WriterClosed,
}

/// Builds the reserved queues and spawns the only task permitted to write plugin stdin.
pub fn spawn_writer<W>(
    stdin: W,
    limits: WriterQueueLimits,
) -> Result<(WriterQueues, mpsc::Receiver<WriterCompletion>), WriterQueueConfigError>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    limits.validate()?;
    let (ordinary_tx, ordinary_rx) = mpsc::channel(limits.ordinary_frames);
    let (cancel_tx, cancel_rx) = mpsc::channel(limits.transport_cancel_frames);
    let (control_tx, control_rx) = mpsc::channel(limits.session_control_frames);
    let (completion_tx, completion_rx) = mpsc::channel(64);
    let queues = WriterQueues {
        ordinary: ordinary_tx,
        transport_cancel: cancel_tx,
        session_control: control_tx,
        ordinary_bytes: Arc::new(Semaphore::new(limits.ordinary_bytes)),
        transport_cancel_bytes: Arc::new(Semaphore::new(limits.transport_cancel_bytes)),
        session_control_bytes: Arc::new(Semaphore::new(limits.session_control_bytes)),
    };
    tokio::spawn(run_writer(
        stdin,
        ordinary_rx,
        cancel_rx,
        control_rx,
        completion_tx,
    ));
    Ok((queues, completion_rx))
}

/// Preserves complete-frame writes while prioritizing non-borrowable control reserves.
async fn run_writer<W>(
    mut stdin: W,
    mut ordinary: mpsc::Receiver<WriterCommand>,
    mut transport_cancel: mpsc::Receiver<WriterCommand>,
    mut session_control: mpsc::Receiver<WriterCommand>,
    completion: mpsc::Sender<WriterCompletion>,
) where
    W: AsyncWrite + Unpin,
{
    loop {
        let command = tokio::select! {
            biased;
            command = session_control.recv() => command,
            command = transport_cancel.recv() => command,
            command = ordinary.recv() => command,
            else => None,
        };
        let Some(command) = command else {
            let _ = stdin.shutdown().await;
            return;
        };

        let result = write_complete_frame(&mut stdin, &command.frame).await;
        let event = match result {
            Ok(()) => WriterCompletion::FrameWritten {
                generation: command.generation,
                owner: command.owner,
            },
            Err(failure) => WriterCompletion::WriteFailed {
                generation: command.generation,
                owner: command.owner,
                bytes_written: failure.bytes_written,
                stage: command.lane.failure_stage(),
                failure: classify_io_error(&failure.error),
            },
        };
        let failed = matches!(event, WriterCompletion::WriteFailed { .. });
        if completion.send(event).await.is_err() || failed {
            return;
        }
    }
}

struct FrameWriteFailure {
    bytes_written: Option<usize>,
    error: io::Error,
}

/// Counts successful bytes so zero is provable while partial/unknown writes remain ambiguous.
async fn write_complete_frame<W>(writer: &mut W, frame: &[u8]) -> Result<(), FrameWriteFailure>
where
    W: AsyncWrite + Unpin,
{
    let mut written = 0_usize;
    while written < frame.len() {
        match writer.write(&frame[written..]).await {
            Ok(0) => {
                return Err(FrameWriteFailure {
                    bytes_written: Some(written),
                    error: io::Error::new(
                        io::ErrorKind::WriteZero,
                        "plugin stdin wrote zero bytes",
                    ),
                });
            }
            Ok(bytes) => written += bytes,
            Err(error) => {
                return Err(FrameWriteFailure {
                    bytes_written: Some(written),
                    error,
                });
            }
        }
    }
    writer.flush().await.map_err(|error| FrameWriteFailure {
        bytes_written: Some(written),
        error,
    })
}

/// FIFO reader events keep all complete frames before the boundary EOF from being overtaken.
#[derive(Debug, Clone, PartialEq)]
pub enum ReaderEvent {
    Envelope(JsonRpcEnvelope),
    BoundaryEof,
    Failure(ReaderFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReaderFailure {
    Io(IoFailure),
    Protocol(ProtocolFailure),
}

/// Spawns the incremental stdout reader without ever buffering the unbounded stream as text.
pub fn spawn_reader<R>(
    stdout: R,
    maximum_json_depth: usize,
    event_capacity: usize,
) -> mpsc::Receiver<ReaderEvent>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let (event_tx, event_rx) = mpsc::channel(event_capacity);
    tokio::spawn(run_reader(stdout, maximum_json_depth, event_tx));
    event_rx
}

async fn run_reader<R>(mut stdout: R, maximum_json_depth: usize, events: mpsc::Sender<ReaderEvent>)
where
    R: AsyncRead + Unpin,
{
    let mut decoder = match FrameDecoder::new(MAX_FRAME_BYTES) {
        Ok(decoder) => decoder,
        Err(_) => return,
    };
    let mut chunk = vec![0_u8; READ_CHUNK_BYTES];
    loop {
        let bytes = match stdout.read(&mut chunk).await {
            Ok(0) => {
                let event = match decoder.finish() {
                    Ok(()) => ReaderEvent::BoundaryEof,
                    Err(_) => {
                        ReaderEvent::Failure(ReaderFailure::Protocol(ProtocolFailure::InvalidFrame))
                    }
                };
                let _ = events.send(event).await;
                return;
            }
            Ok(bytes) => bytes,
            Err(error) => {
                let _ = events
                    .send(ReaderEvent::Failure(ReaderFailure::Io(classify_io_error(
                        &error,
                    ))))
                    .await;
                return;
            }
        };
        let frames = match decoder.decode_chunk(&chunk[..bytes]) {
            Ok(frames) => frames,
            Err(_) => {
                let _ = events
                    .send(ReaderEvent::Failure(ReaderFailure::Protocol(
                        ProtocolFailure::InvalidFrame,
                    )))
                    .await;
                return;
            }
        };
        for frame in frames {
            let envelope = match parse_json_rpc_frame(&frame, maximum_json_depth) {
                Ok(envelope) => envelope,
                Err(_) => {
                    let _ = events
                        .send(ReaderEvent::Failure(ReaderFailure::Protocol(
                            ProtocolFailure::InvalidEnvelope,
                        )))
                        .await;
                    return;
                }
            };
            if events.send(ReaderEvent::Envelope(envelope)).await.is_err() {
                return;
            }
        }
    }
}

/// Fixed-size stderr result retaining only the newest bytes and an exact dropped-byte count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StderrDrainSummary {
    pub retained: Vec<u8>,
    pub dropped_bytes: u64,
    pub failure: Option<IoFailure>,
}

/// Drains stderr immediately into a bounded ring; log publication can never block the pipe.
pub fn spawn_stderr_drain<R>(
    stderr: R,
    retained_bytes: usize,
) -> oneshot::Receiver<StderrDrainSummary>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let (summary_tx, summary_rx) = oneshot::channel();
    tokio::spawn(async move {
        let _ = summary_tx.send(drain_stderr(stderr, retained_bytes).await);
    });
    summary_rx
}

async fn drain_stderr<R>(mut stderr: R, retained_bytes: usize) -> StderrDrainSummary
where
    R: AsyncRead + Unpin,
{
    let mut ring = VecDeque::with_capacity(retained_bytes);
    let mut dropped_bytes = 0_u64;
    let mut chunk = vec![0_u8; READ_CHUNK_BYTES];
    loop {
        match stderr.read(&mut chunk).await {
            Ok(0) => {
                return StderrDrainSummary {
                    retained: ring.into_iter().collect(),
                    dropped_bytes,
                    failure: None,
                };
            }
            Ok(bytes) => {
                for byte in &chunk[..bytes] {
                    if ring.len() == retained_bytes {
                        ring.pop_front();
                        dropped_bytes = dropped_bytes.saturating_add(1);
                    }
                    if retained_bytes > 0 {
                        ring.push_back(*byte);
                    } else {
                        dropped_bytes = dropped_bytes.saturating_add(1);
                    }
                }
            }
            Err(error) => {
                return StderrDrainSummary {
                    retained: ring.into_iter().collect(),
                    dropped_bytes,
                    failure: Some(classify_io_error(&error)),
                };
            }
        }
    }
}

/// Converts OS errors to the bounded metadata categories used by the runtime state machine.
fn classify_io_error(error: &io::Error) -> IoFailure {
    match error.kind() {
        io::ErrorKind::BrokenPipe | io::ErrorKind::WriteZero => IoFailure::BrokenPipe,
        io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted => {
            IoFailure::ConnectionReset
        }
        io::ErrorKind::TimedOut => IoFailure::TimedOut,
        _ => IoFailure::Other,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ora_plugin_protocol::{
        HostRequestId, JsonRpcEnvelope, MAX_FRAME_BYTES, encode_json_frame, encode_json_rpc_request,
    };
    use pretty_assertions::assert_eq;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

    use super::{
        ReaderEvent, SessionControlKind, StderrDrainSummary, WriterCommandOwner, WriterCompletion,
        WriterLane, WriterQueueLimits, spawn_reader, spawn_stderr_drain, spawn_writer,
    };

    /// Proves a response frame remains ahead of EOF even when both are already buffered.
    #[tokio::test]
    async fn reader_preserves_frame_before_boundary_eof() {
        let (mut peer, host) = duplex(1024);
        let id = HostRequestId::from_sequence(1)
            .unwrap_or_else(|error| panic!("test request id: {error}"));
        let payload = serde_json::to_vec(&serde_json::json!({
            "jsonrpc":"2.0",
            "id":id.as_str(),
            "result":{}
        }))
        .unwrap_or_else(|error| panic!("test response JSON: {error}"));
        let frame = encode_json_frame(&payload, MAX_FRAME_BYTES)
            .unwrap_or_else(|error| panic!("test frame: {error}"));
        peer.write_all(&frame)
            .await
            .unwrap_or_else(|error| panic!("write test frame: {error}"));
        peer.shutdown()
            .await
            .unwrap_or_else(|error| panic!("close test peer: {error}"));

        let mut events = spawn_reader(host, 64, 8);
        assert!(matches!(
            events.recv().await,
            Some(ReaderEvent::Envelope(JsonRpcEnvelope::Response(_)))
        ));
        assert_eq!(events.recv().await, Some(ReaderEvent::BoundaryEof));
    }

    /// Confirms writer completion means every frame byte reached the local async pipe.
    #[tokio::test]
    async fn writer_reports_complete_frame_ack() {
        let (host, mut peer) = duplex(1024);
        let (queues, mut completions) = spawn_writer(host, WriterQueueLimits::v1_defaults())
            .unwrap_or_else(|error| panic!("writer queues: {error}"));
        let id = HostRequestId::from_sequence(1)
            .unwrap_or_else(|error| panic!("test request id: {error}"));
        let payload = encode_json_rpc_request(&id, "test", &serde_json::json!({}))
            .unwrap_or_else(|error| panic!("test request JSON: {error}"));
        queues
            .enqueue(
                7,
                WriterCommandOwner::SessionControl(SessionControlKind::Initialize),
                &payload,
                WriterLane::SessionControl,
                Duration::from_secs(1),
            )
            .await
            .unwrap_or_else(|error| panic!("enqueue test frame: {error}"));
        let mut bytes = vec![0_u8; payload.len() + 5];
        peer.read_exact(&mut bytes)
            .await
            .unwrap_or_else(|error| panic!("read test frame: {error}"));
        assert_eq!(
            completions.recv().await,
            Some(WriterCompletion::FrameWritten {
                generation: 7,
                owner: WriterCommandOwner::SessionControl(SessionControlKind::Initialize),
            })
        );
    }

    /// Retains only the newest stderr bytes while continuing to drain a no-newline flood.
    #[tokio::test]
    async fn stderr_ring_is_bounded() {
        let (mut peer, host) = duplex(1024);
        let summary = spawn_stderr_drain(host, 4);
        peer.write_all(b"abcdefgh")
            .await
            .unwrap_or_else(|error| panic!("write stderr flood: {error}"));
        peer.shutdown()
            .await
            .unwrap_or_else(|error| panic!("close stderr peer: {error}"));
        assert_eq!(
            summary
                .await
                .unwrap_or_else(|error| panic!("stderr summary: {error}")),
            StderrDrainSummary {
                retained: b"efgh".to_vec(),
                dropped_bytes: 4,
                failure: None,
            }
        );
    }
}
