//! Stable, secret-free Session MCP setup and refresh errors.

use super::SessionMcpTransportKind;
use ora_contracts::{EmptyErrorParams, PublicError, SessionMcpSetupFailedParams};
use ora_domain::PluginId;
use thiserror::Error;

/// Stable diagnostic code that may appear in UI and logs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionMcpErrorCode {
    LoadCapabilityMissing,
    HttpCapabilityMissing,
    SettingMissing,
    IllegalRuntimeText,
    CommandNotInPackage,
    WorkspaceCwdUnresolved,
    RevisionChanged,
    ConfigurationUnavailable,
    CatalogUnavailable,
}

impl SessionMcpErrorCode {
    /// Wire token used by `PublicError` and operator-facing logs.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::LoadCapabilityMissing => "mcp_load_capability_missing",
            Self::HttpCapabilityMissing => "mcp_http_capability_missing",
            Self::SettingMissing => "mcp_setting_missing",
            Self::IllegalRuntimeText => "mcp_illegal_runtime_text",
            Self::CommandNotInPackage => "mcp_command_not_in_package",
            Self::WorkspaceCwdUnresolved => "mcp_workspace_cwd_unresolved",
            Self::RevisionChanged => "mcp_revision_changed",
            Self::ConfigurationUnavailable => "mcp_configuration_unavailable",
            Self::CatalogUnavailable => "mcp_catalog_unavailable",
        }
    }
}

/// A Session MCP setup or refresh failure that never carries Setting values.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub(crate) enum SessionMcpError {
    #[error("agent does not support session/load required by Ora-managed MCP")]
    LoadCapabilityMissing,
    #[error(
        "agent does not support HTTP MCP for plugin `{plugin_id}` (transport http, {code})",
        code = SessionMcpErrorCode::HttpCapabilityMissing.as_str()
    )]
    HttpCapabilityMissing { plugin_id: PluginId },
    #[error(
        "MCP plugin `{plugin_id}` Setting `{setting_id}` could not be bound (transport {transport}, {code})",
        transport = transport.as_str(),
        code = SessionMcpErrorCode::SettingMissing.as_str()
    )]
    SettingMissing {
        plugin_id: PluginId,
        setting_id: String,
        transport: SessionMcpTransportKind,
    },
    #[error(
        "MCP plugin `{plugin_id}` Setting `{setting_id}` resolved to illegal runtime text (transport {transport}, {code})",
        transport = transport.as_str(),
        code = SessionMcpErrorCode::IllegalRuntimeText.as_str()
    )]
    IllegalRuntimeText {
        plugin_id: PluginId,
        setting_id: String,
        transport: SessionMcpTransportKind,
    },
    #[error(
        "MCP plugin `{plugin_id}` stdio command is not a regular file in the current package ({code})",
        code = SessionMcpErrorCode::CommandNotInPackage.as_str()
    )]
    CommandNotInPackage { plugin_id: PluginId },
    #[error(
        "MCP plugin `{plugin_id}` workspace context could not be resolved to an absolute cwd ({code})",
        code = SessionMcpErrorCode::WorkspaceCwdUnresolved.as_str()
    )]
    WorkspaceCwdUnresolved { plugin_id: PluginId },
    #[error("MCP installed package or configuration revision changed during Session setup")]
    RevisionChanged { plugin_id: Option<PluginId> },
    #[error(
        "MCP plugin `{plugin_id}` configuration could not be re-read ({code})",
        code = SessionMcpErrorCode::ConfigurationUnavailable.as_str()
    )]
    ConfigurationUnavailable { plugin_id: PluginId },
    #[error("installed MCP catalog could not be read")]
    CatalogUnavailable,
}

impl SessionMcpError {
    /// Stable diagnostic code for this failure.
    pub(crate) fn code(&self) -> SessionMcpErrorCode {
        match self {
            Self::LoadCapabilityMissing => SessionMcpErrorCode::LoadCapabilityMissing,
            Self::HttpCapabilityMissing { .. } => SessionMcpErrorCode::HttpCapabilityMissing,
            Self::SettingMissing { .. } => SessionMcpErrorCode::SettingMissing,
            Self::IllegalRuntimeText { .. } => SessionMcpErrorCode::IllegalRuntimeText,
            Self::CommandNotInPackage { .. } => SessionMcpErrorCode::CommandNotInPackage,
            Self::WorkspaceCwdUnresolved { .. } => SessionMcpErrorCode::WorkspaceCwdUnresolved,
            Self::RevisionChanged { .. } => SessionMcpErrorCode::RevisionChanged,
            Self::ConfigurationUnavailable { .. } => SessionMcpErrorCode::ConfigurationUnavailable,
            Self::CatalogUnavailable => SessionMcpErrorCode::CatalogUnavailable,
        }
    }

    fn plugin_id(&self) -> Option<&PluginId> {
        match self {
            Self::LoadCapabilityMissing | Self::CatalogUnavailable => None,
            Self::HttpCapabilityMissing { plugin_id }
            | Self::SettingMissing { plugin_id, .. }
            | Self::IllegalRuntimeText { plugin_id, .. }
            | Self::CommandNotInPackage { plugin_id }
            | Self::WorkspaceCwdUnresolved { plugin_id }
            | Self::ConfigurationUnavailable { plugin_id } => Some(plugin_id),
            Self::RevisionChanged { plugin_id } => plugin_id.as_ref(),
        }
    }

    fn setting_id(&self) -> Option<&str> {
        match self {
            Self::SettingMissing { setting_id, .. }
            | Self::IllegalRuntimeText { setting_id, .. } => Some(setting_id),
            Self::LoadCapabilityMissing
            | Self::HttpCapabilityMissing { .. }
            | Self::CommandNotInPackage { .. }
            | Self::WorkspaceCwdUnresolved { .. }
            | Self::RevisionChanged { .. }
            | Self::ConfigurationUnavailable { .. }
            | Self::CatalogUnavailable => None,
        }
    }

    fn transport(&self) -> Option<SessionMcpTransportKind> {
        match self {
            Self::HttpCapabilityMissing { .. } => Some(SessionMcpTransportKind::Http),
            Self::CommandNotInPackage { .. } => Some(SessionMcpTransportKind::Stdio),
            Self::SettingMissing { transport, .. } | Self::IllegalRuntimeText { transport, .. } => {
                Some(*transport)
            }
            Self::LoadCapabilityMissing
            | Self::WorkspaceCwdUnresolved { .. }
            | Self::RevisionChanged { .. }
            | Self::ConfigurationUnavailable { .. }
            | Self::CatalogUnavailable => None,
        }
    }

    /// Public contract that names Plugin ID, Setting ID, transport, and a stable code only.
    pub(crate) fn public_error(&self) -> PublicError {
        if matches!(self, Self::LoadCapabilityMissing) {
            return PublicError::SessionLoadUnsupported(EmptyErrorParams {});
        }
        PublicError::SessionMcpSetupFailed(Box::new(SessionMcpSetupFailedParams {
            error_code: self.code().as_str().to_string(),
            plugin_id: self.plugin_id().map(PluginId::canonical),
            setting_id: self.setting_id().map(str::to_string),
            transport: self.transport().map(|kind| kind.as_str().to_string()),
        }))
    }

    /// Converts this diagnostic into the backend error the Session actor returns to callers.
    pub(crate) fn into_backend(self) -> crate::BackendError {
        let classification = match self {
            Self::LoadCapabilityMissing | Self::HttpCapabilityMissing { .. } => {
                crate::ErrorClassification::Conflict
            }
            _ => crate::ErrorClassification::Unprocessable,
        };
        crate::BackendError::new(classification, self.public_error(), self.to_string())
    }
}
