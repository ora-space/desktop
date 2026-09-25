//! Connection transports for the Controller–Node protocol.
//!
//! Sessions exchange complete frame bodies (`[type][JSON]`, see `ora-node-protocol`) through
//! [`FrameReceiver`] and [`FrameSender`]; they never see sockets, length prefixes or WebSocket
//! messages. Local deployments use [`ipc`]; cloud deployments reach a Node through a platform
//! WebSocket router with [`websocket`]. The crate knows no sandbox vendor: platform addressing and
//! credentials arrive as plain endpoint data.
use ora_node_protocol::FrameError;
use std::{future::Future, io};
use thiserror::Error;

#[cfg(unix)]
pub mod ipc;
#[cfg(feature = "websocket")]
pub mod websocket;

/// WebSocket close code a Node sends when it already has a live control session. Routers forward
/// close codes unchanged, while they collapse HTTP rejections into gateway errors, so this is the
/// only way the Controller can tell "occupied" from "unreachable" end to end.
pub const CONTROL_SESSION_BUSY_CLOSE_CODE: u16 = 4409;

/// Receiving half of a connection that yields one complete frame body per call.
///
/// Implementations must be cancel-safe: dropping a pending `recv` (for example when a `select!`
/// picks another branch) must neither lose nor split a frame, so sessions can wait for peer
/// messages and timers at the same time without pinning the read future themselves.
pub trait FrameReceiver: Send {
    /// Returns the next frame body, or `None` when the peer ended the connection cleanly.
    fn recv(&mut self) -> impl Future<Output = Result<Option<Vec<u8>>, TransportError>> + Send;
}

/// Sending half of a connection that carries one complete frame body per call.
///
/// Sending is bounded: a slow peer blocks the caller, who applies its own deadline, instead of
/// the transport buffering without limit.
pub trait FrameSender: Send {
    /// Sends and flushes one frame body produced by the protocol encoder.
    fn send(&mut self, frame: Vec<u8>) -> impl Future<Output = Result<(), TransportError>> + Send;
}

/// Accepts inbound control connections for a Node's single listening entry.
///
/// The Node keeps the one-live-session rule itself and only asks the acceptor to either open a
/// connection into a frame channel or reject it as busy, so the rule is identical for every
/// transport. `accept` must be cancel-safe because the Node races it against shutdown and against
/// the live session.
pub trait Acceptor: Sync {
    /// An accepted connection that has not completed any transport handshake yet.
    type Pending: Send + 'static;
    type Receiver: FrameReceiver + 'static;
    type Sender: FrameSender + 'static;

    /// Waits for the next inbound connection.
    fn accept(&self) -> impl Future<Output = io::Result<Self::Pending>> + Send;

    /// Completes the transport handshake and splits the connection into frame halves.
    fn open(
        &self,
        pending: Self::Pending,
    ) -> impl Future<Output = Result<(Self::Receiver, Self::Sender), TransportError>> + Send;

    /// Refuses a connection while another session is live. The future owns everything it needs so
    /// the Node can run it off the session's path under its own deadline.
    fn reject_busy(&self, pending: Self::Pending) -> impl Future<Output = ()> + Send + 'static;
}

/// A connection failure after it was established; callers treat every variant as "connection
/// unavailable", never as an execution outcome.
#[derive(Debug, Error)]
pub enum TransportError {
    #[error("transport I/O failed")]
    Io(#[from] io::Error),
    #[error("invalid frame envelope")]
    Frame(#[from] FrameError),
    /// The peer (or a router relaying it) closed with a non-normal close code.
    #[error("peer closed the connection with code {code}: {reason}")]
    Closed { code: u16, reason: String },
    /// Only binary messages carry frames; anything else is a protocol violation.
    #[error("peer sent a message that is not a binary frame")]
    UnexpectedMessage,
    #[cfg(feature = "websocket")]
    #[error("WebSocket connection failed")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),
}

impl TransportError {
    /// Whether the Node refused this connection because another control session is live.
    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Closed { code, .. } if *code == CONTROL_SESSION_BUSY_CLOSE_CODE)
    }
}

/// Why an outbound connection could not be established. The class only guides logs and future
/// sandbox lifecycle decisions; every class is retried and none releases execution responsibility.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectFailure {
    /// No listener, network failure, timeout, or a router that could not reach the Node.
    Unreachable,
    /// A router reported that the addressed sandbox does not exist.
    NotFound,
    /// The endpoint or its credentials were refused, or the endpoint itself is malformed.
    Rejected,
}

/// An outbound connection attempt that failed before any frame could be exchanged.
#[derive(Debug, Error)]
#[error("connection {failure:?}: {source}")]
pub struct ConnectError {
    pub failure: ConnectFailure,
    #[source]
    pub source: Box<dyn std::error::Error + Send + Sync>,
}

impl ConnectError {
    /// Wraps a cause under its failure class.
    pub fn new(
        failure: ConnectFailure,
        source: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        Self {
            failure,
            source: source.into(),
        }
    }
}
