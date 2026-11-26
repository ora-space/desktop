use agent_client_protocol_schema::v1::RequestId;
use agent_client_protocol_schema::v1::RequestPermissionRequest;
use agent_client_protocol_schema::v1::SessionId;
use agent_client_protocol_schema::v1::SessionNotification;
use serde_json::Value;

use crate::error::AcpError;
use crate::pending::PendingResponse;

/// Carries an ACP extension notification (`_`-prefixed method) in wire order.
///
/// The ACP specification reserves `_`-prefixed methods for custom extensions (`extNotification`
/// in the SDK). Keeping the method name and raw JSON payload intact here lets `ora-acp` stay
/// provider-neutral while preserving transport order beside standard `session/update` frames.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionNotification {
    pub method: String,
    pub params: Value,
}

/// Carries one permission request together with its JSON-RPC correlation id.
#[derive(Debug, Clone, PartialEq)]
pub struct PermissionRequest {
    pub request_id: RequestId,
    pub request: RequestPermissionRequest,
}

/// Carries a response that terminates one ordered session request.
#[derive(Debug)]
pub struct SessionResponse {
    pub(crate) request_id: RequestId,
    pub(crate) session_id: SessionId,
    pub(crate) response: PendingResponse,
}

impl SessionResponse {
    /// Identifies the provider session that owns this response.
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }
}

/// Where one frame fell in the order the reader took frames off the connection.
///
/// A direct response bypasses the ordered inbound stream, so comparing its order with an event's is
/// the only way to tell which of the two the agent sent first. Orders are only comparable within
/// one connection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReadOrder(u64);

impl ReadOrder {
    /// Returns the order of the frame read right after this one.
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

/// One inbound event together with the order its frame was read in.
#[derive(Debug)]
pub struct ReadEvent {
    pub order: ReadOrder,
    pub event: AcpInboundEvent,
}

/// Preserves wire order for all events that participate in a session turn.
#[derive(Debug)]
pub enum AcpInboundEvent {
    SessionUpdate(SessionNotification),
    ExtensionNotification(ExtensionNotification),
    PermissionRequest(PermissionRequest),
    SessionResponse(SessionResponse),
    Fatal(AcpError),
}
