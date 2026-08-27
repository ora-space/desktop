use std::future::Future;

use serde_json::Value;
use tokio::sync::mpsc;

use crate::error::AcpError;

/// Yields whole inbound ACP messages, or the framing failure that ended the connection.
///
/// The stream is unbounded because it is connection-wide: bounding it would let one busy session
/// stall every other session sharing the same agent process.
pub type AcpMessages = mpsc::UnboundedReceiver<Result<Value, AcpError>>;

/// Carries whole ACP JSON-RPC messages for one connection.
///
/// Implementations own framing and ordering: `send` must deliver complete messages in call order,
/// and the receiver handed to `AcpPeer::spawn` must yield exactly one message per frame. The peer
/// never inspects transport-level framing, so a transport is free to carry messages any way it
/// likes as long as those two guarantees hold.
pub trait AcpTransport: Send + Sync + 'static {
    fn send(&self, message: Value) -> impl Future<Output = Result<(), AcpError>> + Send;
}
