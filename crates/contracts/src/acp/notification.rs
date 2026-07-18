use serde::{Deserialize, Serialize};
use serde_with::{serde_as, skip_serializing_none};
use ts_rs::TS;

use super::{common::SessionId, rpc::RequestId, session::SessionUpdate};

/// Notification containing a session update from the agent.
///
/// Used to stream real-time progress and results during prompt processing.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export_to = "acp/notification.ts")]
#[serde(rename_all = "camelCase")]
pub struct SessionNotification {
    /// The ID of the session this update pertains to.
    pub session_id: SessionId,
    /// The actual update content.
    pub update: SessionUpdate,
}

impl SessionNotification {
    /// Builds [`SessionNotification`] with the required notification fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(session_id: impl Into<SessionId>, update: SessionUpdate) -> Self {
        Self {
            session_id: session_id.into(),
            update,
        }
    }
}

/// Notification to cancel ongoing operations for a session.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/notification.ts")]
#[serde(rename_all = "camelCase")]
pub struct CancelNotification {
    /// The ID of the session to cancel operations for.
    pub session_id: SessionId,
}

impl CancelNotification {
    /// Builds [`CancelNotification`] with the required notification fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(session_id: impl Into<SessionId>) -> Self {
        Self {
            session_id: session_id.into(),
        }
    }
}

/// Notification to cancel an ongoing request.
///
/// See protocol docs: [Cancellation](https://agentclientprotocol.com/protocol/cancellation)
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CancelRequestNotification {
    /// The ID of the request to cancel.
    pub request_id: RequestId,
}

impl CancelRequestNotification {
    /// Builds [`CancelRequestNotification`] with the required notification fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(request_id: impl Into<RequestId>) -> Self {
        Self {
            request_id: request_id.into(),
        }
    }
}

/// Exports every TypeScript binding declared in this module into the target directory.
///
/// `CancelRequestNotification` is intentionally not exported: it depends on `rpc::RequestId`,
/// which is not part of the frontend SDK yet.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    SessionNotification::export(config)?;
    CancelNotification::export(config)?;
    Ok(())
}
