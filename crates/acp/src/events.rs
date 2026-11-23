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

/// Preserves wire order for all events that participate in a session turn.
#[derive(Debug)]
pub enum AcpInboundEvent {
    SessionUpdate(SessionNotification),
    ExtensionNotification(ExtensionNotification),
    PermissionRequest(PermissionRequest),
    SessionResponse(SessionResponse),
    Fatal(AcpError),
}
