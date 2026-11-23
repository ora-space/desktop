//! Memory-only lifecycle requests. These are never durable execution commands.

use super::validation::{validate_execution_ids, validate_identity, validate_protocol_version};
use crate::{
    ExecutionId, MAX_SESSION_MCP_COUNTER, MessageValidationError, NodeId, NodeRuntimeIdentity,
    OperationId, PluginId, ProtocolVersion, RequestId, SessionMcpErrorCode, SessionMcpState,
    SessionMcpUpdateMember, ValidateMessage,
};
use serde::{Deserialize, Serialize};

/// Shared envelope for correlated lifecycle observations and ephemeral inputs, without sequence.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionMcpMessage<P> {
    pub protocol_version: ProtocolVersion,
    pub operation_id: OperationId,
    pub execution_id: ExecutionId,
    pub payload: P,
}

impl<P: ValidateMessage> ValidateMessage for SessionMcpMessage<P> {
    /// Validates identity before the receiver can inspect or resolve member settings.
    fn validate(&self) -> Result<(), MessageValidationError> {
        validate_protocol_version(self.protocol_version)?;
        validate_execution_ids(&self.operation_id, &self.execution_id)?;
        self.payload.validate()
    }
}

/// A whole authorized set; success means desired advanced, not that ACP has received it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateSessionMcp {
    pub node_id: NodeId,
    pub request_id: RequestId,
    pub generation: u64,
    pub members: Vec<SessionMcpUpdateMember>,
}

/// Read-only observation; querying cannot start a resolver, Agent, refresh or health probe.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GetSessionMcpState {
    pub node_id: NodeId,
    pub request_id: RequestId,
}

/// A user retry is ordered within the current generation and selected member.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeSessionMcp {
    pub node_id: NodeId,
    pub request_id: RequestId,
    pub generation: u64,
    pub plugin_id: PluginId,
    pub probe_attempt: u64,
}

/// Business refusals keep the connection alive and never imply persisted input or completion.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMcpResult {
    Accepted,
    Duplicate,
    Stale,
    Conflict,
    NotFound,
    SessionEnded,
    NotEnabled,
    InvalidSelection,
    NotConfigured,
    CapacityExceeded,
}

/// Value-free current state; an unavailable session supplies no historical health snapshot.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionMcpReply {
    pub node: NodeRuntimeIdentity,
    pub request_id: RequestId,
    pub result: SessionMcpResult,
    /// Input conflicts are reply-local; they must not overwrite the desired generation's failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<SessionMcpErrorCode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<SessionMcpState>,
}

pub type UpdateSessionMcpMessage = SessionMcpMessage<UpdateSessionMcp>;
pub type GetSessionMcpStateMessage = SessionMcpMessage<GetSessionMcpState>;
pub type ProbeSessionMcpMessage = SessionMcpMessage<ProbeSessionMcp>;
pub type SessionMcpReplyMessage = SessionMcpMessage<SessionMcpReply>;

/// A finite correlation key prevents unbounded retained request metadata.
fn validate_target(node: &NodeId, request: &RequestId) -> Result<(), MessageValidationError> {
    validate_identity(node.is_empty(), "node_id")?;
    validate_identity(request.is_empty(), "request_id")?;
    if request.as_str().len() > 128 {
        return Err(MessageValidationError::InvalidSessionMcp);
    }
    Ok(())
}

impl ValidateMessage for UpdateSessionMcp {
    /// New updates cannot target the generation-zero startup input.
    fn validate(&self) -> Result<(), MessageValidationError> {
        validate_target(&self.node_id, &self.request_id)?;
        if !(1..=MAX_SESSION_MCP_COUNTER).contains(&self.generation) {
            return Err(MessageValidationError::InvalidSessionMcp);
        }
        crate::domain::validate_update_members(&self.members)
    }
}

impl ValidateMessage for GetSessionMcpState {
    /// Validation has no read or refresh side effects.
    fn validate(&self) -> Result<(), MessageValidationError> {
        validate_target(&self.node_id, &self.request_id)
    }
}

impl ValidateMessage for ProbeSessionMcp {
    /// Generation zero is probeable, but zero is reserved for automatic probe attempts.
    fn validate(&self) -> Result<(), MessageValidationError> {
        validate_target(&self.node_id, &self.request_id)?;
        if ora_domain::PluginId::parse(self.plugin_id.as_str()).is_err() {
            return Err(MessageValidationError::InvalidSessionMcp);
        }
        if self.generation > MAX_SESSION_MCP_COUNTER
            || !(1..=MAX_SESSION_MCP_COUNTER).contains(&self.probe_attempt)
        {
            return Err(MessageValidationError::InvalidSessionMcp);
        }
        Ok(())
    }
}

impl ValidateMessage for SessionMcpReply {
    /// An unknown or ended execution must never return another execution's snapshot.
    fn validate(&self) -> Result<(), MessageValidationError> {
        self.node
            .validate()
            .map_err(|field| MessageValidationError::EmptyField { field })?;
        validate_target(&self.node.node_id, &self.request_id)?;
        if self.state.is_some()
            && matches!(
                self.result,
                SessionMcpResult::NotFound
                    | SessionMcpResult::SessionEnded
                    | SessionMcpResult::NotEnabled
                    | SessionMcpResult::CapacityExceeded
            )
        {
            return Err(MessageValidationError::InvalidSessionMcp);
        }
        if self.code.is_some()
            && (self.result != SessionMcpResult::Conflict
                || self.code != Some(SessionMcpErrorCode::ConfigurationRevisionConflict))
        {
            return Err(MessageValidationError::InvalidSessionMcp);
        }
        if let Some(state) = &self.state {
            state.validate()?;
        }
        Ok(())
    }
}
