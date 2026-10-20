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
    RuntimeControl,
    /// `InstallPlugins` and `RemovePlugins`.
    PluginInstall,
    /// `StartAgentSession`, Thread events and session commands.
    AgentSession,
    /// Scoped model access through the platform gateway; no direct upstream credentials.
    ModelProxy,
    /// `DeliverRevision` and the upload grant exchange.
    RevisionDelivery,
    /// Sessions with a prior Revision, restored before the agent starts, and the download grant
    /// exchange; a delivery of such a session reports an unchanged prior final commit as unchanged.
    RevisionRestore,
}

/// Controller greeting used to negotiate a protocol version for a new session.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Hello {
    /// Optional live-preview opt-in; omitted by older Controllers.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub thread_previews: bool,
    pub controller_id: ControllerId,
    pub supported_versions: Vec<ProtocolVersion>,
}

/// Node response binding a negotiated session to persistent and incarnation identities.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct HelloAccepted {
    /// True only when both peers opted into ephemeral preview messages.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub thread_previews: bool,
    pub selected_version: ProtocolVersion,
    pub node: NodeRuntimeIdentity,
    pub capabilities: Vec<NodeCapability>,
    /// Optional, exact declared-model adapter capability; older Nodes honestly omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_adapter: Option<ModelAdapterCapabilities>,
}

/// The execution target and protocol surface validated by this Node incarnation at startup.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelAdapterCapabilities {
    pub contract_version: u32,
    pub target: String,
    pub protocols: Vec<ora_plugin_protocol::AgentModelProtocol>,
}

impl ModelAdapterCapabilities {
    /// Compares immutable execution requirements against this incarnation's advertised surface.
    pub fn supports(&self, target: &str, version: u32, protocol: &str) -> bool {
        self.target == target
            && self.contract_version == version
            && self
                .protocols
                .iter()
                .any(|accepted| accepted.as_str() == protocol)
    }
}

/// Liveness signal from the current Node incarnation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Heartbeat {
    pub node: NodeRuntimeIdentity,
}

/// Liveness signal from the owning Controller. It lets the Node keep a per-frame read deadline
/// as Controller liveness while the session is idle, without routing through execution admission.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ControllerHeartbeat {
    pub controller_id: ControllerId,
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

/// Complete Controller heartbeat envelope, including only metadata valid for this message.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ControllerHeartbeatMessage {
    pub protocol_version: ProtocolVersion,
    pub payload: ControllerHeartbeat,
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
        if let Some(adapter) = &payload.model_adapter
            && (!payload.capabilities.contains(&NodeCapability::ModelProxy)
                || adapter.contract_version != 1
                || !matches!(
                    adapter.target.as_str(),
                    "x86_64-unknown-linux-gnu" | "aarch64-unknown-linux-gnu"
                )
                || adapter.protocols.is_empty()
                || adapter.protocols.len() > 2
                || (adapter.protocols.len() == 2 && adapter.protocols[0] == adapter.protocols[1]))
        {
            return Err(MessageValidationError::EmptyField {
                field: "model_adapter",
            });
        }
        if payload.thread_previews && !payload.capabilities.contains(&NodeCapability::AgentSession)
        {
            return Err(MessageValidationError::InvalidThreadPreview);
        }
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

impl ValidateMessage for ControllerHeartbeatMessage {
    /// Enforces this message’s semantic rules on both send and receive.
    fn validate(&self) -> Result<(), MessageValidationError> {
        let Self {
            protocol_version,
            payload,
        } = self;

        validate_protocol_version(*protocol_version)?;
        validate_identity(payload.controller_id.is_empty(), "controller_id")
    }
}
