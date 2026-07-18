use std::sync::Arc;

use derive_more::{Display, From};
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, skip_serializing_none};
use ts_rs::TS;

use super::{common::SessionId, tool_call::ToolCallUpdate};

/// Request for user permission to execute a tool call.
///
/// Sent when the agent needs authorization before performing a sensitive operation.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export_to = "acp/permission.ts")]
#[serde(rename_all = "camelCase")]
pub struct RequestPermissionRequest {
    /// The session ID for this request.
    pub session_id: SessionId,
    /// Details about the tool call requiring permission.
    pub tool_call: ToolCallUpdate,
    /// Available permission options for the user to choose from.
    pub options: Vec<PermissionOption>,
}

impl RequestPermissionRequest {
    /// Builds [`RequestPermissionRequest`] with the required request fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(
        session_id: impl Into<SessionId>,
        tool_call: ToolCallUpdate,
        options: Vec<PermissionOption>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            tool_call,
            options,
        }
    }
}

/// An option presented to the user when requesting permission.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/permission.ts")]
#[serde(rename_all = "camelCase")]
pub struct PermissionOption {
    /// Unique identifier for this permission option.
    pub option_id: PermissionOptionId,
    /// Human-readable label to display to the user.
    pub name: String,
    /// Hint about the nature of this permission option.
    pub kind: PermissionOptionKind,
}

impl PermissionOption {
    /// Builds [`PermissionOption`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(
        option_id: impl Into<PermissionOptionId>,
        name: impl Into<String>,
        kind: PermissionOptionKind,
    ) -> Self {
        Self {
            option_id: option_id.into(),
            name: name.into(),
            kind,
        }
    }
}

/// Unique identifier for a permission option.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Display, From, TS)]
#[serde(transparent)]
#[from(Arc<str>, String, &'static str)]
#[ts(type = "string", export_to = "acp/permission.ts")]
pub struct PermissionOptionId(pub Arc<str>);

impl PermissionOptionId {
    /// Wraps a protocol string as a typed [`PermissionOptionId`].
    #[must_use]
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        Self(id.into())
    }
}

/// The type of permission option being presented to the user.
///
/// Helps clients choose appropriate icons and UI treatment.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/permission.ts")]
#[serde(rename_all = "snake_case")]
pub enum PermissionOptionKind {
    /// Allow this operation only this time.
    AllowOnce,
    /// Allow this operation and remember the choice.
    AllowAlways,
    /// Reject this operation only this time.
    RejectOnce,
    /// Reject this operation and remember the choice.
    RejectAlways,
}

/// Response to a permission request.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/permission.ts")]
#[serde(rename_all = "camelCase")]
pub struct RequestPermissionResponse {
    /// The user's decision on the permission request.
    // This extra-level is unfortunately needed because the output must be an object
    pub outcome: RequestPermissionOutcome,
}

impl RequestPermissionResponse {
    /// Builds [`RequestPermissionResponse`] with the required response fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(outcome: RequestPermissionOutcome) -> Self {
        Self { outcome }
    }
}

/// The outcome of a permission request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/permission.ts")]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RequestPermissionOutcome {
    /// The prompt turn was cancelled before the user responded.
    ///
    /// When a client sends a `session/cancel` notification to cancel an ongoing
    /// prompt turn, it MUST respond to all pending `session/request_permission`
    /// requests with this `Cancelled` outcome.
    Cancelled,
    /// The user selected one of the provided options.
    Selected(SelectedPermissionOutcome),
}

/// The user selected one of the provided options.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/permission.ts")]
#[serde(rename_all = "camelCase")]
pub struct SelectedPermissionOutcome {
    /// The ID of the option the user selected.
    pub option_id: PermissionOptionId,
}

impl SelectedPermissionOutcome {
    /// Builds [`SelectedPermissionOutcome`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(option_id: impl Into<PermissionOptionId>) -> Self {
        Self {
            option_id: option_id.into(),
        }
    }
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    PermissionOptionId::export(config)?;
    PermissionOptionKind::export(config)?;
    PermissionOption::export(config)?;
    SelectedPermissionOutcome::export(config)?;
    RequestPermissionOutcome::export(config)?;
    RequestPermissionResponse::export(config)?;
    RequestPermissionRequest::export(config)?;
    Ok(())
}
