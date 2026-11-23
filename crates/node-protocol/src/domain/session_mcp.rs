//! Explicit, bounded MCP input. Values are serialized only by the control frame, never Debug.
use crate::{MessageValidationError, PluginId, PluginVersion};
use ora_plugin_config::SettingValue;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Limits process fan-out from a single session setup.
pub const MAX_SESSION_MCP_SERVERS: usize = 16;
/// Total compact JSON bytes of all settings maps, including keys and escaping.
pub const MAX_SESSION_MCP_SETTINGS_BYTES: usize = 64 * 1024;

/// Transport metadata safe to report beside an MCP setup failure.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMcpTransport {
    Stdio,
    Http,
}

impl SessionMcpTransport {
    /// Stable lowercase spelling used by logs and the Cloud mapping.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Http => "http",
        }
    }
}

/// One exact installed package with session-local scalar values; never a Node-local command.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionMcpBinding {
    pub plugin_id: PluginId,
    pub plugin_version: PluginVersion,
    pub settings: BTreeMap<String, SettingValue>,
}

impl fmt::Debug for SessionMcpBinding {
    /// A start command may be debugged, so values must be redacted at their owning boundary.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionMcpBinding")
            .field("plugin_id", &self.plugin_id)
            .field("plugin_version", &self.plugin_version)
            .field("settings", &"[REDACTED]")
            .finish()
    }
}

/// Rejects ambiguous selection instead of silently sorting, deduplicating or normalizing it.
pub(super) fn validate_bindings(
    bindings: &[SessionMcpBinding],
) -> Result<(), MessageValidationError> {
    if bindings.len() > MAX_SESSION_MCP_SERVERS {
        return Err(MessageValidationError::InvalidSessionMcp);
    }
    let mut previous: Option<&str> = None;
    let mut bytes = 0usize;
    for binding in bindings {
        let id = binding.plugin_id.as_str();
        if ora_domain::PluginId::parse(id).is_err()
            || previous.is_some_and(|previous| previous >= id)
            || !semver::Version::parse(binding.plugin_version.as_str())
                .is_ok_and(|version| version.to_string() == binding.plugin_version.as_str())
        {
            return Err(MessageValidationError::InvalidSessionMcp);
        }
        previous = Some(id);
        bytes += serde_json::to_vec(&binding.settings)
            .map_err(|_| MessageValidationError::InvalidSessionMcp)?
            .len();
        if bytes > MAX_SESSION_MCP_SETTINGS_BYTES {
            return Err(MessageValidationError::InvalidSessionMcp);
        }
    }
    Ok(())
}
