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
    #[serde(flatten)]
    pub selection: WorkflowDebugSelection,
    pub base_revision: u32,
    pub patch: Vec<WorkflowDebugPatch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "workflow-run.ts")]
#[serde(rename_all = "camelCase")]
pub enum WorkflowDebugSelection {
    NodeRunId(String),
    PendingTarget(PendingWorkflowDebugTarget),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "workflow-run.ts")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PendingWorkflowDebugTarget {
    pub root_scope_id: String,
    pub scope_id: String,
    pub node_id: String,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_node_run_id: Option<String>,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
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
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_run_id: Option<String>,
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_scope_id: Option<String>,
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
    /// Earlier physical attempt supplying dispatch input for a waiting retry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub input_source_node_run_id: Option<String>,
    pub preview_id: String,
    pub base_revision: u32,
    pub target: WorkflowDebugTarget,
    /// Effective member configurations after rebuilding a selected container from its entrance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "Record<string, unknown>")]
    pub member_configurations: Option<std::collections::BTreeMap<String, serde_json::Value>>,
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
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum WorkflowDebugPhase {
    Stopping,
    /// The attempts listed here could not be proven stopped or isolated; they keep the fence.
    StopFailed {
        reason: String,
        unsettled_node_run_ids: Vec<String>,
    },
    Applied {
        revision: u32,
    },
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
    /// First attempt the scheduler admitted for a `pendingTarget` after the revision applied.
    /// Null until then, and always null for an operation that selected an existing attempt.
    pub first_node_run_id: Option<String>,
    /// Run status and error as they were when the confirmation was accepted. A terminal run keeps
    /// them while the stop is in progress and returns to `running` once the revision applies.
    pub prior_run_status: crate::WorkflowRunStatus,
    pub prior_error: Option<String>,
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
    WorkflowDebugSelection::export(config)?;
    PendingWorkflowDebugTarget::export(config)?;
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
