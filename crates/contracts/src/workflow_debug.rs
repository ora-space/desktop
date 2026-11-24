//! Run-local debug configuration, preview and durable confirmation receipts.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "workflow-run.ts")]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkflowDebugPatch {
    Replace {
        path: String,
        #[ts(type = "unknown")]
        value: serde_json::Value,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "workflow-run.ts")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewWorkflowDebugRequest {
    pub run_id: String,
    pub node_run_id: String,
    pub base_revision: u32,
    pub patch: Vec<WorkflowDebugPatch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "workflow-run.ts")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommitWorkflowDebugRequest {
    pub run_id: String,
    pub preview_id: String,
    pub request_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "workflow-run.ts")]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDebugTarget {
    pub node_run_id: String,
    pub scope_id: String,
    pub node_id: String,
    pub container_node_run_id: Option<String>,
    pub round: Option<u32>,
    pub index: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "workflow-run.ts")]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDebugImpact {
    pub keep: Vec<String>,
    pub invalidate: Vec<String>,
    pub stop: Vec<String>,
    pub schedule: Vec<String>,
    pub predicted_branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "workflow-run.ts")]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDebugPreview {
    pub preview_id: String,
    pub base_revision: u32,
    pub target: WorkflowDebugTarget,
    #[serde(flatten)]
    pub impact: WorkflowDebugImpact,
    pub workspace: WorkflowDebugWorkspace,
    pub external_actions_may_repeat: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "workflow-run.ts")]
#[serde(rename_all = "snake_case")]
pub enum WorkflowDebugWorkspace {
    Keep,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "workflow-run.ts")]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum WorkflowDebugPhase {
    Stopping,
    StopFailed { reason: String },
    Applied { revision: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "workflow-run.ts")]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDebugOperation {
    pub operation_id: String,
    pub request_id: String,
    pub base_revision: u32,
    #[serde(flatten)]
    pub phase: WorkflowDebugPhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "workflow-run.ts")]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDebugHistoryEntry {
    pub operation: WorkflowDebugOperation,
    pub preview: WorkflowDebugPreview,
    pub patch: Vec<WorkflowDebugPatch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "workflow-run.ts")]
#[serde(rename_all = "camelCase")]
pub struct GetWorkflowDebugHistoryRequest {
    pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "workflow-run.ts")]
#[serde(rename_all = "camelCase")]
pub struct GetWorkflowDebugHistoryResponse {
    pub entries: Vec<WorkflowDebugHistoryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "workflow-run.ts")]
#[serde(rename_all = "camelCase")]
pub struct GetWorkflowDebugAttemptRequest {
    pub run_id: String,
    pub node_run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "workflow-run.ts")]
#[serde(rename_all = "camelCase")]
pub struct GetWorkflowDebugAttemptResponse {
    pub target: WorkflowDebugTarget,
    pub revision: Option<u32>,
    pub snapshot_id: Option<String>,
    #[ts(type = "unknown | null")]
    pub configuration: Option<serde_json::Value>,
    pub status: crate::WorkflowNodeStatus,
    pub superseded: bool,
    pub retired_by_operation_id: Option<String>,
    pub input_recorded: bool,
    pub selected_branch: Option<String>,
    pub output: Option<String>,
    pub error: Option<String>,
}

/// Emits the shared workflow-run wire types from their Rust declarations.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    WorkflowDebugPatch::export(config)?;
    PreviewWorkflowDebugRequest::export(config)?;
    CommitWorkflowDebugRequest::export(config)?;
    WorkflowDebugTarget::export(config)?;
    WorkflowDebugImpact::export(config)?;
    WorkflowDebugPreview::export(config)?;
    WorkflowDebugWorkspace::export(config)?;
    WorkflowDebugPhase::export(config)?;
    WorkflowDebugOperation::export(config)?;
    WorkflowDebugHistoryEntry::export(config)?;
    GetWorkflowDebugHistoryRequest::export(config)?;
    GetWorkflowDebugHistoryResponse::export(config)?;
    GetWorkflowDebugAttemptRequest::export(config)?;
    GetWorkflowDebugAttemptResponse::export(config)?;
    Ok(())
}
