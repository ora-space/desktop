use super::validation::{
    MessageValidationError, ValidateMessage, validate_identity, validate_protocol_version,
};
use crate::{ControllerId, NodeRuntimeIdentity, ProtocolVersion};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Capability names negotiated during the Controller–Node handshake.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeCapability {
    WorktreeExecution,
    RepositoryClone,
}

/// Controller greeting used to negotiate a protocol version for a new session.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Hello {
    pub controller_id: ControllerId,
    pub supported_versions: Vec<ProtocolVersion>,
}

/// Node response binding a negotiated session to persistent and incarnation identities.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct HelloAccepted {
    pub selected_version: ProtocolVersion,
    pub node: NodeRuntimeIdentity,
    pub capabilities: Vec<NodeCapability>,
}

/// Liveness signal from the current Node incarnation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Heartbeat {
    pub node: NodeRuntimeIdentity,
}

/// Complete Hello envelope, including only metadata valid for this message.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HelloMessage {
    pub protocol_version: ProtocolVersion,
    pub payload: Hello,
}

impl ValidateMessage for HelloMessage {
    /// Enforces this message’s semantic rules on both send and receive.
    fn validate(&self) -> Result<(), MessageValidationError> {
        let Self {
            protocol_version,
            payload,
        } = self;

        validate_protocol_version(*protocol_version)?;
        validate_identity(payload.controller_id.is_empty(), "controller_id")?;
        if payload.supported_versions.is_empty() {
            return Err(MessageValidationError::NoSupportedProtocolVersions);
        }
        let mut versions = HashSet::with_capacity(payload.supported_versions.len());
        for version in &payload.supported_versions {
            if !versions.insert(*version) {
                return Err(MessageValidationError::DuplicateProtocolVersion {
                    version: version.value(),
                });
            }
        }
        if !versions.contains(protocol_version) {
            return Err(MessageValidationError::EnvelopeVersionNotAdvertised {
                version: protocol_version.value(),
            });
        }
        Ok(())
    }
}

/// Complete HelloAccepted envelope, including only metadata valid for this message.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HelloAcceptedMessage {
    pub protocol_version: ProtocolVersion,
    pub payload: HelloAccepted,
}

/// Complete Heartbeat envelope, including only metadata valid for this message.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HeartbeatMessage {
    pub protocol_version: ProtocolVersion,
    pub payload: Heartbeat,
}

impl ValidateMessage for HelloAcceptedMessage {
    /// Enforces this message’s semantic rules on both send and receive.
    fn validate(&self) -> Result<(), MessageValidationError> {
        let Self {
            protocol_version,
            payload,
        } = self;

        validate_protocol_version(*protocol_version)?;
        payload
            .node
            .validate()
            .map_err(|field| MessageValidationError::EmptyField { field })?;
        if payload.selected_version != *protocol_version {
            return Err(MessageValidationError::SelectedVersionMismatch {
                selected: payload.selected_version.value(),
                envelope: protocol_version.value(),
            });
        }
        let mut capabilities = HashSet::with_capacity(payload.capabilities.len());
        for capability in &payload.capabilities {
            if !capabilities.insert(*capability) {
                return Err(MessageValidationError::DuplicateCapability);
            }
        }
        if capabilities.is_empty() {
            return Err(MessageValidationError::NoExecutionCapabilities);
        }
        Ok(())
    }
}

impl ValidateMessage for HeartbeatMessage {
    /// Enforces this message’s semantic rules on both send and receive.
    fn validate(&self) -> Result<(), MessageValidationError> {
        let Self {
            protocol_version,
            payload,
        } = self;

        validate_protocol_version(*protocol_version)?;
        payload
            .node
            .validate()
            .map_err(|field| MessageValidationError::EmptyField { field })
    }
}
