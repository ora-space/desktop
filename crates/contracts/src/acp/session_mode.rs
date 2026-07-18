use super::common::Meta;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A session mode identifier.
pub type SessionModeId = String;

/// The current mode and available modes for a session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/session_mode.ts")]
pub struct SessionModeState {
    /// The active mode identifier.
    pub current_mode_id: SessionModeId,
    /// The modes available to the agent.
    pub available_modes: Vec<SessionMode>,
    /// Optional extension data.
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    #[ts(type = "Record<string, unknown>")]
    #[ts(optional)]
    pub meta: Option<Meta>,
}

/// A selectable agent session mode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/session_mode.ts")]
pub struct SessionMode {
    /// The stable mode identifier.
    pub id: SessionModeId,
    /// The display name.
    pub name: String,
    /// An optional mode description.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub description: Option<String>,
    /// Optional extension data.
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    #[ts(type = "Record<string, unknown>")]
    #[ts(optional)]
    pub meta: Option<Meta>,
}

/// Parameters for the `session/set_mode` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/session_mode.ts")]
pub struct SetSessionModeParams {
    /// The target session.
    pub session_id: String,
    /// The mode to activate.
    pub mode_id: SessionModeId,
    /// Optional extension data.
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    #[ts(type = "Record<string, unknown>")]
    #[ts(optional)]
    pub meta: Option<Meta>,
}
