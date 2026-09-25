use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Public clone intent; target Node and deployment paths belong to server configuration.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "controller-api.ts")]
pub struct MiniCloneRequest {
    pub request_id: String,
    pub repository: String,
    pub branch: String,
}

/// Stable receipt for durable acceptance, independent of eventual execution success.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "controller-api.ts")]
pub struct MiniCloneAccepted {
    pub request_id: String,
    pub operation_id: String,
    pub execution_id: String,
}

/// Presentation of durable facts; pending does not assert that Node is connected or running Git.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "controller-api.ts")]
pub enum MiniCloneState {
    Pending,
    Succeeded {
        path: String,
        commit: String,
    },
    Failed {
        reason: MiniCloneFailure,
        retained_path: Option<String>,
    },
}

/// Known clone failure categories, without raw Git output or deployment credentials.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "controller-api.ts")]
pub enum MiniCloneFailure {
    SourceUnavailable,
    BranchNotFound,
    DestinationConflict,
    OperationFailed,
    Interrupted,
}

/// One durable operation for list and detail views; paths remain Node-local facts.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "controller-api.ts")]
pub struct MiniCloneOperation {
    pub operation_id: String,
    pub execution_id: String,
    pub node_id: String,
    pub repository: String,
    pub branch: String,
    pub state: MiniCloneState,
}

/// HTTP failures do not become terminal execution failures.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "controller-api.ts")]
pub enum MiniErrorCode {
    InvalidInput,
    Conflict,
    NotFound,
    Unavailable,
}

/// Stable error envelope for the non-production HTTP adapter.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[ts(export_to = "controller-api.ts")]
pub struct MiniError {
    pub code: MiniErrorCode,
}

/// Generates transitional Controller API DTOs without installing Desktop operations or bindings.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    MiniCloneRequest::export(config)?;
    MiniCloneAccepted::export(config)?;
    MiniCloneOperation::export_all(config)?;
    MiniError::export_all(config)?;
    Ok(())
}
