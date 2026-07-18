//! Dedicated session mode methods will be removed in a future version of the protocol.
//! Use Session Config Options instead.

use std::sync::Arc;

use derive_more::{Display, From};
use serde::{Deserialize, Serialize};
use serde_with::{DefaultOnError, serde_as, skip_serializing_none};
use ts_rs::TS;

use super::{common::SessionId, serde_util::IntoOption};

/// Unique identifier for a Session Mode.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, From, Display, TS)]
#[serde(transparent)]
#[from(Arc<str>, String, &'static str)]
#[ts(type = "string", export_to = "acp/session_mode.ts")]
pub struct SessionModeId(pub Arc<str>);

impl SessionModeId {
    /// Wraps a protocol string as a typed [`SessionModeId`].
    #[must_use]
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        Self(id.into())
    }
}

/// The set of modes and the one currently active.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/session_mode.ts")]
pub struct SessionModeState {
    /// The current mode the Agent is in.
    pub current_mode_id: SessionModeId,
    /// The set of modes that the Agent can operate in
    #[serde_as(deserialize_as = "DefaultOnError")]
    pub available_modes: Vec<SessionMode>,
}

impl SessionModeState {
    /// Builds [`SessionModeState`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(
        current_mode_id: impl Into<SessionModeId>,
        available_modes: Vec<SessionMode>,
    ) -> Self {
        Self {
            current_mode_id: current_mode_id.into(),
            available_modes,
        }
    }
}

/// A mode the agent can operate in.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/session_mode.ts")]
pub struct SessionMode {
    /// Stable identifier used to refer to this protocol object in later messages.
    pub id: SessionModeId,
    /// Human-readable name shown for this protocol object.
    pub name: String,
    /// Optional human-readable details shown with this protocol object.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub description: Option<String>,
}

impl SessionMode {
    /// Builds [`SessionMode`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(id: impl Into<SessionModeId>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: None,
        }
    }

    /// Sets or clears the optional `description` field.
    #[must_use]
    pub fn description(mut self, description: impl IntoOption<String>) -> Self {
        self.description = description.into_option();
        self
    }
}

/// Request parameters for setting a session mode.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/session_mode.ts")]
pub struct SetSessionModeRequest {
    /// The ID of the session to set the mode for.
    pub session_id: SessionId,
    /// The ID of the mode to set.
    pub mode_id: SessionModeId,
}

impl SetSessionModeRequest {
    /// Builds [`SetSessionModeRequest`] with the required request fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(session_id: impl Into<SessionId>, mode_id: impl Into<SessionModeId>) -> Self {
        Self {
            session_id: session_id.into(),
            mode_id: mode_id.into(),
        }
    }
}

/// Response to `session/set_mode` method.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "acp/session_mode.ts")]
pub struct SetSessionModeResponse {}

impl SetSessionModeResponse {
    /// Builds [`SetSessionModeResponse`] with the required response fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    SessionModeId::export(config)?;
    SessionModeState::export(config)?;
    SessionMode::export(config)?;
    SetSessionModeRequest::export(config)?;
    SetSessionModeResponse::export(config)?;
    Ok(())
}
