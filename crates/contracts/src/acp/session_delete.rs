use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Identifies the session removed by `session/delete`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/session_delete.ts")]
pub struct SessionDeleteRequest {
    pub session_id: String,
}

/// Represents the empty successful result of `session/delete`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/session_delete.ts")]
pub struct SessionDeleteResponse {}
