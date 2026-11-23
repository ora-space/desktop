//! Lifecycle inputs carry values only in configured members; observations have no value fields.

use crate::{
    MAX_SESSION_MCP_SERVERS, MAX_SESSION_MCP_SETTINGS_BYTES, MessageValidationError, PluginId,
    PluginVersion, SessionMcpTransport, SettingValue, ValidateMessage,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt};

/// JSON clients can compare these counters exactly; neither end may wrap or round them.
pub const MAX_SESSION_MCP_COUNTER: u64 = (1u64 << 53) - 1;

/// A selected member that currently cannot supply a complete binding.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMcpUnavailable {
    McpPluginMissing,
    McpSettingMissing,
}

/// One member of the complete, immutable authorization set. Unavailable is not removal.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum SessionMcpUpdateMember {
    Configured {
        plugin_id: PluginId,
        plugin_version: PluginVersion,
        configuration_revision: u64,
        settings: BTreeMap<String, SettingValue>,
    },
    Unavailable {
        plugin_id: PluginId,
        code: SessionMcpUnavailable,
    },
}

impl fmt::Debug for SessionMcpUpdateMember {
    /// Settings stay redacted even when an entire control request is formatted.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionMcpUpdateMember")
            .field("identity", &self.identity())
            .field("settings", &"[REDACTED]")
            .finish()
    }
}

impl SessionMcpUpdateMember {
    /// The identity is present even when the selected package is no longer available.
    pub fn plugin_id(&self) -> &PluginId {
        match self {
            Self::Configured { plugin_id, .. } | Self::Unavailable { plugin_id, .. } => plugin_id,
        }
    }

    /// Separates values from identity before any queue, observation or diagnostic boundary.
    pub fn identity(&self) -> SessionMcpMemberIdentity {
        match self {
            Self::Configured {
                plugin_id,
                plugin_version,
                configuration_revision,
                ..
            } => SessionMcpMemberIdentity::Configured {
                plugin_id: plugin_id.clone(),
                plugin_version: plugin_version.clone(),
                configuration_revision: *configuration_revision,
            },
            Self::Unavailable { plugin_id, code } => SessionMcpMemberIdentity::Unavailable {
                plugin_id: plugin_id.clone(),
                code: *code,
            },
        }
    }
}

/// A value-free projection, intentionally incapable of representing Settings or their hashes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum SessionMcpMemberIdentity {
    Configured {
        plugin_id: PluginId,
        plugin_version: PluginVersion,
        configuration_revision: u64,
    },
    Unavailable {
        plugin_id: PluginId,
        code: SessionMcpUnavailable,
    },
}

impl SessionMcpMemberIdentity {
    /// Returns the immutable authorization key without exposing a configured binding.
    pub fn plugin_id(&self) -> &PluginId {
        match self {
            Self::Configured { plugin_id, .. } | Self::Unavailable { plugin_id, .. } => plugin_id,
        }
    }
}

/// Generation participates in identity even when the member bindings have not changed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionMcpRevisionIdentity {
    pub generation: u64,
    pub members: Vec<SessionMcpMemberIdentity>,
}

/// A query reports convergence independently of configuration and host health.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMcpConvergence {
    Pending,
    Refreshing,
    Current,
    Blocked,
    Stopping,
    Ended,
}

/// Closed error vocabulary: resolved commands, credentials and third-party messages cannot fit.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMcpErrorCode {
    ConfigurationRevisionConflict,
    McpPluginMissing,
    McpSettingMissing,
    McpHttpCapabilityMissing,
    McpLoadSessionMissing,
    McpRefreshFailed,
    McpRefreshTimeout,
}

/// Probe failures cannot be confused with setup or ACP convergence failures.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMcpProbeErrorCode {
    McpSpawnFailed,
    McpExitedPrematurely,
    McpHandshakeFailed,
    McpProbeTimeout,
    McpToolsUnavailable,
    McpHttpUnreachable,
    McpHttpUnauthorized,
    McpHttpServerError,
}

/// Optional safe identifiers narrow a failure without serializing the underlying error.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionMcpFailure {
    pub code: SessionMcpErrorCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<PluginId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setting_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<SessionMcpTransport>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMcpUnknownReason {
    NotProbed,
    ContextMissing,
}

/// Health proves a host handshake only; it cannot authorize prompt admission.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum SessionMcpHealth {
    Healthy,
    Unhealthy { code: SessionMcpProbeErrorCode },
    Unknown { reason: SessionMcpUnknownReason },
}

/// Resource flags distinguish queued work and retained cleanup from endpoint failure.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionMcpMemberState {
    pub plugin_id: PluginId,
    pub health: SessionMcpHealth,
    pub probe_attempt: u64,
    pub probe_in_flight: bool,
    pub queued: bool,
    pub cleanup_pending: bool,
}

/// Only this session and an anonymous capacity fact cross its observation boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionMcpState {
    pub state_version: u64,
    pub desired: SessionMcpRevisionIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<SessionMcpRevisionIdentity>,
    pub convergence: SessionMcpConvergence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<SessionMcpFailure>,
    pub members: Vec<SessionMcpMemberState>,
    pub probe_capacity_available: bool,
}

impl ValidateMessage for SessionMcpState {
    /// Rejects impossible observations as well as counters JSON consumers cannot compare exactly.
    fn validate(&self) -> Result<(), MessageValidationError> {
        let invalid = || MessageValidationError::InvalidSessionMcp;
        self.desired.validate()?;
        if let Some(active) = &self.active {
            active.validate()?;
            if active.generation > self.desired.generation
                || active
                    .members
                    .iter()
                    .any(|member| matches!(member, SessionMcpMemberIdentity::Unavailable { .. }))
                || !active
                    .members
                    .iter()
                    .map(SessionMcpMemberIdentity::plugin_id)
                    .eq(self
                        .desired
                        .members
                        .iter()
                        .map(SessionMcpMemberIdentity::plugin_id))
            {
                return Err(invalid());
            }
        }
        if self.state_version > MAX_SESSION_MCP_COUNTER
            || self.convergence == SessionMcpConvergence::Ended
            || (self.convergence == SessionMcpConvergence::Current
                && self.active.as_ref() != Some(&self.desired))
            || !self.members.iter().map(|member| &member.plugin_id).eq(self
                .desired
                .members
                .iter()
                .map(SessionMcpMemberIdentity::plugin_id))
            || self.members.iter().any(|member| {
                member.probe_attempt > MAX_SESSION_MCP_COUNTER
                    || (member.cleanup_pending && !member.probe_in_flight)
            })
        {
            return Err(invalid());
        }
        if let Some(id) = self
            .failure
            .as_ref()
            .and_then(|failure| failure.plugin_id.as_ref())
            && !self
                .desired
                .members
                .iter()
                .any(|member| member.plugin_id() == id)
        {
            return Err(invalid());
        }
        Ok(())
    }
}

impl ValidateMessage for SessionMcpRevisionIdentity {
    /// Validates a value-free projection with the same selection rules as input.
    fn validate(&self) -> Result<(), MessageValidationError> {
        if self.generation > MAX_SESSION_MCP_COUNTER || self.members.len() > MAX_SESSION_MCP_SERVERS
        {
            return Err(MessageValidationError::InvalidSessionMcp);
        }
        let mut previous: Option<&str> = None;
        for member in &self.members {
            let id = member.plugin_id().as_str();
            if ora_domain::PluginId::parse(id).is_err()
                || previous.is_some_and(|previous| previous >= id)
            {
                return Err(MessageValidationError::InvalidSessionMcp);
            }
            previous = Some(id);
            if let SessionMcpMemberIdentity::Configured {
                plugin_version,
                configuration_revision,
                ..
            } = member
                && (*configuration_revision > MAX_SESSION_MCP_COUNTER
                    || !semver::Version::parse(plugin_version.as_str())
                        .is_ok_and(|version| version.to_string() == plugin_version.as_str()))
            {
                return Err(MessageValidationError::InvalidSessionMcp);
            }
        }
        Ok(())
    }
}

/// Validates complete input shape without attempting package resolution or retaining values.
pub(crate) fn validate_update_members(
    members: &[SessionMcpUpdateMember],
) -> Result<(), MessageValidationError> {
    let invalid = || MessageValidationError::InvalidSessionMcp;
    if members.len() > MAX_SESSION_MCP_SERVERS {
        return Err(invalid());
    }
    let mut previous: Option<&str> = None;
    let mut bytes = 0;
    for member in members {
        let id = member.plugin_id().as_str();
        if ora_domain::PluginId::parse(id).is_err()
            || previous.is_some_and(|previous| previous >= id)
        {
            return Err(invalid());
        }
        previous = Some(id);
        if let SessionMcpUpdateMember::Configured {
            plugin_version,
            configuration_revision,
            settings,
            ..
        } = member
        {
            if *configuration_revision > MAX_SESSION_MCP_COUNTER
                || !semver::Version::parse(plugin_version.as_str())
                    .is_ok_and(|version| version.to_string() == plugin_version.as_str())
            {
                return Err(invalid());
            }
            bytes += serde_json::to_vec(settings).map_err(|_| invalid())?.len();
            if bytes > MAX_SESSION_MCP_SETTINGS_BYTES {
                return Err(invalid());
            }
        }
    }
    Ok(())
}
