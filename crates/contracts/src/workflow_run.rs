use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

/// Describes the lifecycle state of a workflow run in the public contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub enum WorkflowRunStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    /// Derived on the wire when a `Running` run has at least one awaiting (interactive) node;
    /// the persisted run status stays `Running` so cancel/restart semantics are unchanged.
    AwaitingInput,
}

/// Describes the lifecycle state of one node execution in the public contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub enum WorkflowNodeStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

/// Public workflow run payload without persistence audit metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct WorkflowRun {
    pub id: String,
    pub workspace_id: String,
    pub workflow_id: String,
    pub snapshot_id: String,
    pub name: String,
    pub status: WorkflowRunStatus,
    pub state: Option<String>,
    pub input: Option<String>,
    pub output: Option<String>,
    pub error: Option<String>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Public node-run payload without persistence audit metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct WorkflowNodeRun {
    pub id: String,
    pub run_id: String,
    pub scope_id: String,
    pub node_id: String,
    pub node_type: String,
    pub session_id: Option<String>,
    pub status: WorkflowNodeStatus,
    pub input: Option<String>,
    pub output: Option<String>,
    pub error: Option<String>,
    pub payload: Option<String>,
    /// Composite-region round this row executed in; `null` for outer rows. A region node holds
    /// one row per round, so the run view groups states by `(node_id, iteration)`.
    #[serde(default)]
    pub iteration: Option<u32>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Lifecycle state of one persisted Loop round.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub enum WorkflowExecutionScopeStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

/// Public Loop-round identity used to group repeated node definitions in run history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct WorkflowExecutionScope {
    pub id: String,
    pub run_id: String,
    pub parent_loop_node_run_id: String,
    pub round_index: u32,
    pub status: WorkflowExecutionScopeStatus,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Lightweight run summary for list views with direct workspace ownership.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct WorkflowRunSummary {
    pub id: String,
    pub name: String,
    pub workspace_id: String,
    pub project_id: String,
    pub workflow_id: String,
    /// The published snapshot version the run froze, distinguishing runs created from different
    /// versions of the same workflow.
    pub version: String,
    pub status: WorkflowRunStatus,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub created_at: i64,
}

/// Identifies the Ora display language frozen for generated workflow-run prompts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "workflow-run.ts")]
pub enum WorkflowRunLocale {
    #[serde(rename = "zh-CN")]
    #[ts(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "en-US")]
    #[ts(rename = "en-US")]
    EnUs,
}

// ── Create ──

/// Carries the fields required to create a workflow run against a published snapshot and workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct CreateWorkflowRunRequest {
    pub workspace_id: String,
    pub workflow_id: String,
    pub locale: WorkflowRunLocale,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub kickoff_input: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// `None` means inject last-failure context (the same as `Some(true)`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub inject_last_failure: Option<bool>,
}

/// Returns the created workspace-owned run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct CreateWorkflowRunResponse {
    pub run: WorkflowRun,
}

// ── Get by ID ──

/// Identifies the workflow run to retrieve by its stable identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct GetWorkflowRunRequest {
    pub run_id: String,
}

/// Returns the full run detail including its display name and node runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct GetWorkflowRunResponse {
    pub run: WorkflowRun,
    pub name: String,
    pub workspace_id: String,
    pub project_id: String,
    pub nodes: Vec<WorkflowNodeRun>,
    /// Loop round identities; internal variable pools and branch decisions remain private.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub scopes: Option<Vec<WorkflowExecutionScope>>,
    /// Typed variable-pool projection; persistence metadata remains internal.
    pub variables: Vec<WorkflowRunVariable>,
    /// Condition decisions keyed by node id for branch-aware rendering.
    pub condition_decisions: BTreeMap<String, String>,
    /// Earlier attempts that failed and were run again (automatic retry, resume) or whose run
    /// was restarted, oldest first. `nodes` holds only the latest attempt of each node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub failed_attempts: Option<Vec<WorkflowNodeFailedAttempt>>,
}

/// One earlier failed attempt of a node, taken from its persisted `payload.error_detail`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct WorkflowNodeFailedAttempt {
    /// Id of the attempt's (soft-deleted) node run.
    pub node_run_id: String,
    pub node_id: String,
    pub scope_id: String,
    /// Composite-region round of the attempt; `null` for outer and Loop rows.
    pub iteration: Option<u32>,
    /// The attempt's session, whose transcript remains readable.
    pub session_id: Option<String>,
    /// Same numbering as `error_detail.attempt`: 1 for the node's first attempt in the run.
    pub attempt: u32,
    /// Failure kind (snake_case, as in `error_detail.kind`).
    pub kind: String,
    /// Top-level failure message; for session failures this is generic, and the agent's own
    /// reason is in `source_chain`.
    pub message: String,
    /// Source chain of the originating error, outermost first (as in
    /// `error_detail.source_chain`); empty when the engine raised the failure itself.
    pub source_chain: Vec<String>,
    /// Unix millis the failure was recorded.
    pub recorded_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

/// One declared run variable and its optional current value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct WorkflowRunVariable {
    pub selector: Vec<String>,
    pub value_type: String,
    pub source_node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "unknown")]
    pub value: Option<serde_json::Value>,
}

// ── List by project ──

/// Requests the workflow run summaries for one project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct ListWorkflowRunsRequest {
    pub project_id: String,
}

/// Returns the visible run summaries for the project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct ListWorkflowRunsResponse {
    pub runs: Vec<WorkflowRunSummary>,
}

// ── List by workflow ──

/// Requests the workflow run summaries for one workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct ListWorkflowRunsByWorkflowRequest {
    pub workflow_id: String,
}

/// Returns the visible run summaries for the workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct ListWorkflowRunsByWorkflowResponse {
    pub runs: Vec<WorkflowRunSummary>,
}

// ── List node runs ──

/// Identifies the run whose node-run history to retrieve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct ListWorkflowNodeRunsRequest {
    pub run_id: String,
}

/// Returns the node-run records of one run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct ListWorkflowNodeRunsResponse {
    pub nodes: Vec<WorkflowNodeRun>,
}

// ── Delete ──

/// Identifies the workflow run to soft-delete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct DeleteWorkflowRunRequest {
    pub run_id: String,
}

/// Returns the identifier of the soft-deleted run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct DeleteWorkflowRunResponse {
    pub run_id: String,
}

/// Identifies the workflow run whose Workspace-owned display name should change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct RenameWorkflowRunRequest {
    pub run_id: String,
    pub name: String,
}

/// Returns the workflow run after its display name was replaced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct RenameWorkflowRunResponse {
    pub run: WorkflowRun,
}

// ── Start / Cancel / Restart (execution engine) ──

/// Identifies the run to start executing against its frozen snapshot graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct StartWorkflowRunRequest {
    pub run_id: String,
}

/// Returns the run after starting (or idempotently its current state).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct StartWorkflowRunResponse {
    pub run: WorkflowRun,
}

/// Identifies the running run whose node sessions should be stopped and run cancelled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct CancelWorkflowRunRequest {
    pub run_id: String,
}

/// Returns the cancelled run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct CancelWorkflowRunResponse {
    pub run: WorkflowRun,
}

/// Identifies the non-running run to reset and re-run from its start node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct RestartWorkflowRunRequest {
    pub run_id: String,
}

/// Returns the reset and re-running run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct RestartWorkflowRunResponse {
    pub run: WorkflowRun,
}

/// How the worktree is treated before a failed run is resumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "workflow-run.ts")]
pub enum ResumeRollbackMode {
    Keep,
    NodeFiles,
    Checkpoint,
}

/// Identifies the failed or cancelled run to resume from its failed nodes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct ResumeWorkflowRunRequest {
    pub run_id: String,
    /// `None` keeps the worktree as it is.
    #[serde(default)]
    #[ts(optional)]
    pub rollback: Option<ResumeRollbackMode>,
    /// `None` keeps the run on its current snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub snapshot_id: Option<String>,
}

/// Returns the resumed and re-running run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct ResumeWorkflowRunResponse {
    pub run: WorkflowRun,
    pub pre_rollback_checkpoint: Option<String>,
}

/// Identifies the failed or cancelled run whose resume preview should be loaded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct PreviewWorkflowRunResumeRequest {
    pub run_id: String,
}

/// One file's incremental change, matching the node payload `file_changes` shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct WorkflowFileChange {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
}

/// Preview of one failed or cancelled node that would be re-run.
///
/// When automatic retries replaced earlier attempts of the node since the last start, restart,
/// or resume, the whole chain is one rollback unit: `started_at` and `checkpoint` are those of
/// its first attempt, `node_file_changes` lists the files every attempt changed (line counts
/// summed), and `checkpoint_error` is the first one any attempt recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct ResumeFailedNodePreview {
    pub node_id: String,
    pub node_run_id: String,
    pub started_at: Option<i64>,
    pub checkpoint: Option<String>,
    pub checkpoint_error: Option<String>,
    /// What the node itself recorded (`payload.file_changes` of the failed attempts).
    pub node_file_changes: Vec<WorkflowFileChange>,
    /// Live diff of the worktree against this node's checkpoint (includes edits made after the failure).
    pub changed_since_checkpoint: Vec<WorkflowFileChange>,
    /// Owning composite node id (Iteration or Loop) when this row belongs to a composite resume
    /// unit: a region member, a Loop body node, or the composite itself.
    #[ts(optional)]
    pub resume_unit_node_id: Option<String>,
}

/// Describes whether a run can be resumed and which rollback modes are available.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct PreviewWorkflowRunResumeResponse {
    /// Run is failed/cancelled, no running node, and at least one failed/cancelled node.
    pub resumable: bool,
    pub failed_nodes: Vec<ResumeFailedNodePreview>,
    /// Every failed node has a checkpoint, and so did every attempt of its retry chain that ran.
    pub node_files_available: bool,
    /// `"no_file_changes"` when a failed node has no checkpoint / recorded changes;
    /// `"composite_region"` when the resume unit is a composite (Iteration or Loop).
    pub node_files_unavailable_reason: Option<String>,
    /// Available when the run is resumable, the resume unit has a checkpoint, and no live node
    /// run outside that unit was still active after the unit's earliest start (`finished_at` is
    /// none or later than that instant, or `started_at` is later). Start/Condition/Output rows
    /// that finished before the unit started do not count.
    pub checkpoint_available: bool,
    /// `"no_checkpoint"` | `"siblings_ran_after_checkpoint"` | `"not_resumable"`.
    pub checkpoint_unavailable_reason: Option<String>,
    pub current_snapshot_id: String,
    pub current_snapshot_version: String,
    pub published_snapshot_id: Option<String>,
    pub published_snapshot_version: Option<String>,
    pub published_snapshot_switchable: bool,
    pub published_snapshot_incompatible_reason: Option<String>,
}

/// Sets the kickoff input of a pending run, used as the start node's input on start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct UpdateWorkflowRunInputRequest {
    pub run_id: String,
    pub input: Option<String>,
    /// Start-variable values keyed by their declared short name; JSON null clears an assignment.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[ts(optional, type = "Record<string, unknown>")]
    pub variables: BTreeMap<String, serde_json::Value>,
}

/// Returns the run with its updated input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct UpdateWorkflowRunInputResponse {
    pub run: WorkflowRun,
}

// ── Complete workflow node (interactive fallback) ──

/// Who requested the completion of one workflow node.
///
/// Phase 1 carries only the human path; the agent/CLI path reuses the same command later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub enum NodeCompletionRequester {
    Human,
}

/// Identifies the awaiting interactive node to complete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct CompleteWorkflowNodeRequest {
    pub run_id: String,
    pub node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub requester: Option<NodeCompletionRequester>,
}

/// Returns the run after the node completed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct CompleteWorkflowNodeResponse {
    pub run: WorkflowRun,
}

/// Identifies the failed agent node whose one-off AI diagnosis should be generated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct DiagnoseWorkflowNodeFailureRequest {
    pub run_id: String,
    pub node_id: String,
}

/// Plain-text diagnosis stored on the node run as `payload.ai_diagnosis` and shown as an AI guess.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct WorkflowNodeAiDiagnosis {
    pub text: String,
    pub agent_cli: String,
    pub model: String,
    pub generated_at: i64,
}

/// Returns the generated diagnosis; nothing in scheduling or resume reads this value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow-run.ts")]
pub struct DiagnoseWorkflowNodeFailureResponse {
    pub diagnosis: WorkflowNodeAiDiagnosis,
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    WorkflowRunStatus::export(config)?;
    WorkflowNodeStatus::export(config)?;
    WorkflowRun::export(config)?;
    WorkflowNodeRun::export(config)?;
    WorkflowExecutionScopeStatus::export(config)?;
    WorkflowExecutionScope::export(config)?;
    WorkflowRunVariable::export(config)?;
    WorkflowRunSummary::export(config)?;
    WorkflowRunLocale::export(config)?;
    CreateWorkflowRunRequest::export(config)?;
    CreateWorkflowRunResponse::export(config)?;
    GetWorkflowRunRequest::export(config)?;
    GetWorkflowRunResponse::export(config)?;
    WorkflowNodeFailedAttempt::export(config)?;
    ListWorkflowRunsRequest::export(config)?;
    ListWorkflowRunsResponse::export(config)?;
    ListWorkflowRunsByWorkflowRequest::export(config)?;
    ListWorkflowRunsByWorkflowResponse::export(config)?;
    ListWorkflowNodeRunsRequest::export(config)?;
    ListWorkflowNodeRunsResponse::export(config)?;
    DeleteWorkflowRunRequest::export(config)?;
    DeleteWorkflowRunResponse::export(config)?;
    RenameWorkflowRunRequest::export(config)?;
    RenameWorkflowRunResponse::export(config)?;
    StartWorkflowRunRequest::export(config)?;
    StartWorkflowRunResponse::export(config)?;
    CancelWorkflowRunRequest::export(config)?;
    CancelWorkflowRunResponse::export(config)?;
    RestartWorkflowRunRequest::export(config)?;
    RestartWorkflowRunResponse::export(config)?;
    ResumeRollbackMode::export(config)?;
    ResumeWorkflowRunRequest::export(config)?;
    ResumeWorkflowRunResponse::export(config)?;
    PreviewWorkflowRunResumeRequest::export(config)?;
    WorkflowFileChange::export(config)?;
    ResumeFailedNodePreview::export(config)?;
    PreviewWorkflowRunResumeResponse::export(config)?;
    UpdateWorkflowRunInputRequest::export(config)?;
    UpdateWorkflowRunInputResponse::export(config)?;
    NodeCompletionRequester::export(config)?;
    CompleteWorkflowNodeRequest::export(config)?;
    CompleteWorkflowNodeResponse::export(config)?;
    DiagnoseWorkflowNodeFailureRequest::export(config)?;
    WorkflowNodeAiDiagnosis::export(config)?;
    DiagnoseWorkflowNodeFailureResponse::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        CompleteWorkflowNodeRequest, CompleteWorkflowNodeResponse, CreateWorkflowRunRequest,
        CreateWorkflowRunResponse, DeleteWorkflowRunRequest, DeleteWorkflowRunResponse,
        DiagnoseWorkflowNodeFailureRequest, GetWorkflowRunRequest, GetWorkflowRunResponse,
        ListWorkflowNodeRunsRequest, ListWorkflowNodeRunsResponse,
        ListWorkflowRunsByWorkflowRequest, ListWorkflowRunsByWorkflowResponse,
        ListWorkflowRunsRequest, ListWorkflowRunsResponse, NodeCompletionRequester,
        ResumeRollbackMode, ResumeWorkflowRunRequest, WorkflowExecutionScope,
        WorkflowExecutionScopeStatus, WorkflowNodeAiDiagnosis, WorkflowNodeRun, WorkflowNodeStatus,
        WorkflowRun, WorkflowRunLocale, WorkflowRunStatus, WorkflowRunSummary, WorkflowRunVariable,
    };
    use pretty_assertions::assert_eq;
    use serde::Serialize;
    use serde_json::{Value, json};
    use std::collections::BTreeMap;

    /// Verifies the workflow-run contracts serialize to frontend-friendly camelCase payloads.
    #[test]
    fn serializes_workflow_run_contracts() {
        let run = WorkflowRun {
            id: "run-1".to_string(),
            workspace_id: "workspace-1".to_string(),
            workflow_id: "workflow-1".to_string(),
            snapshot_id: "snapshot-1".to_string(),
            name: "Workflow workflow-1 30".to_string(),
            status: WorkflowRunStatus::Pending,
            state: Some("{\"current_nodes\":[]}".to_string()),
            input: Some("kickoff".to_string()),
            output: None,
            error: None,
            started_at: None,
            finished_at: None,
            created_at: 30,
            updated_at: 30,
        };
        let node = WorkflowNodeRun {
            id: "node-1".to_string(),
            run_id: "run-1".to_string(),
            scope_id: "root:run-1".to_string(),
            node_id: "start".to_string(),
            node_type: "start".to_string(),
            session_id: None,
            status: WorkflowNodeStatus::Succeeded,
            input: None,
            output: None,
            error: None,
            payload: None,
            iteration: None,
            started_at: Some(30),
            finished_at: Some(31),
            created_at: 30,
            updated_at: 31,
        };
        let scope = WorkflowExecutionScope {
            id: "round-1".into(),
            run_id: "run-1".into(),
            parent_loop_node_run_id: "loop-node-run".into(),
            round_index: 1,
            status: WorkflowExecutionScopeStatus::Succeeded,
            created_at: 31,
            updated_at: 32,
        };
        // A region row carries its round on the wire; the outer form omits the field entirely.
        let region_node = WorkflowNodeRun {
            id: "node-2".to_string(),
            scope_id: "root:run-1".into(),
            run_id: "run-1".to_string(),
            node_id: "fix".to_string(),
            node_type: "agent".to_string(),
            session_id: None,
            status: WorkflowNodeStatus::Succeeded,
            input: None,
            output: None,
            error: None,
            payload: None,
            iteration: Some(2),
            started_at: Some(32),
            finished_at: Some(33),
            created_at: 32,
            updated_at: 33,
        };

        assert_serialized_json(
            &run,
            json!({
                "id": "run-1",
                "workspaceId": "workspace-1",
                "workflowId": "workflow-1",
                "snapshotId": "snapshot-1",
                "name": "Workflow workflow-1 30",
                "status": "pending",
                "state": "{\"current_nodes\":[]}",
                "input": "kickoff",
                "output": null,
                "error": null,
                "startedAt": null,
                "finishedAt": null,
                "createdAt": 30,
                "updatedAt": 30,
            }),
        );
        assert_serialized_json(
            &CreateWorkflowRunRequest {
                workspace_id: "workspace-1".to_string(),
                workflow_id: "workflow-1".to_string(),
                locale: WorkflowRunLocale::ZhCn,
                snapshot_id: None,
                kickoff_input: None,
                name: None,
                inject_last_failure: None,
            },
            json!({
                "workspaceId": "workspace-1",
                "workflowId": "workflow-1",
                "locale": "zh-CN"
            }),
        );
        assert_serialized_json(
            &CreateWorkflowRunResponse { run: run.clone() },
            json!({
                "run": {
                    "id": "run-1",
                    "workspaceId": "workspace-1",
                    "workflowId": "workflow-1",
                    "snapshotId": "snapshot-1",
                    "name": "Workflow workflow-1 30",
                    "status": "pending",
                    "state": "{\"current_nodes\":[]}",
                    "input": "kickoff",
                    "output": null,
                    "error": null,
                    "startedAt": null,
                    "finishedAt": null,
                    "createdAt": 30,
                    "updatedAt": 30,
                },
            }),
        );
        assert_serialized_json(
            &GetWorkflowRunRequest {
                run_id: "run-1".to_string(),
            },
            json!({ "runId": "run-1" }),
        );
        assert_serialized_json(
            &GetWorkflowRunResponse {
                run: run.clone(),
                name: "Workflow workflow-1 30".to_string(),
                workspace_id: "workspace-1".to_string(),
                project_id: "project-1".to_string(),
                nodes: vec![node.clone()],
                scopes: Some(vec![scope]),
                variables: vec![WorkflowRunVariable {
                    selector: vec!["start".to_string(), "count".to_string()],
                    value_type: "integer".to_string(),
                    source_node_id: "start".to_string(),
                    value: Some(json!(3)),
                }],
                condition_decisions: BTreeMap::from([(
                    "condition-1".to_string(),
                    "case-1".to_string(),
                )]),
                failed_attempts: None,
            },
            json!({
                "run": {
                    "id": "run-1",
                    "workspaceId": "workspace-1",
                    "workflowId": "workflow-1",
                    "snapshotId": "snapshot-1",
                    "name": "Workflow workflow-1 30",
                    "status": "pending",
                    "state": "{\"current_nodes\":[]}",
                    "input": "kickoff",
                    "output": null,
                    "error": null,
                    "startedAt": null,
                    "finishedAt": null,
                    "createdAt": 30,
                    "updatedAt": 30,
                },
                "name": "Workflow workflow-1 30",
                "workspaceId": "workspace-1",
                "projectId": "project-1",
                "nodes": [{
                    "id": "node-1",
                    "runId": "run-1",
                    "scopeId": "root:run-1",
                    "nodeId": "start",
                    "nodeType": "start",
                    "sessionId": null,
                    "status": "succeeded",
                    "input": null,
                    "output": null,
                    "error": null,
                    "payload": null,
                    "iteration": null,
                    "startedAt": 30,
                    "finishedAt": 31,
                    "createdAt": 30,
                    "updatedAt": 31,
                }],
                "scopes": [{
                    "id": "round-1",
                    "runId": "run-1",
                    "parentLoopNodeRunId": "loop-node-run",
                    "roundIndex": 1,
                    "status": "succeeded",
                    "createdAt": 31,
                    "updatedAt": 32,
                }],
                "variables": [{
                    "selector": ["start", "count"],
                    "valueType": "integer",
                    "sourceNodeId": "start",
                    "value": 3,
                }],
                "conditionDecisions": { "condition-1": "case-1" },
            }),
        );
        // Region rows serialize their round so run views can group by (nodeId, iteration).
        assert_serialized_json(
            &region_node,
            json!({
                "id": "node-2",
                "runId": "run-1",
                "scopeId": "root:run-1",
                "nodeId": "fix",
                "nodeType": "agent",
                "sessionId": null,
                "status": "succeeded",
                "input": null,
                "output": null,
                "error": null,
                "payload": null,
                "iteration": 2,
                "startedAt": 32,
                "finishedAt": 33,
                "createdAt": 32,
                "updatedAt": 33,
            }),
        );
        assert_serialized_json(
            &ListWorkflowRunsRequest {
                project_id: "project-1".to_string(),
            },
            json!({ "projectId": "project-1" }),
        );
        assert_serialized_json(
            &ListWorkflowRunsResponse {
                runs: vec![WorkflowRunSummary {
                    id: "run-1".to_string(),
                    name: "Workflow workflow-1 30".to_string(),
                    workspace_id: "workspace-1".to_string(),
                    project_id: "project-1".to_string(),
                    workflow_id: "workflow-1".to_string(),
                    version: "v30".to_string(),
                    status: WorkflowRunStatus::Pending,
                    started_at: None,
                    finished_at: None,
                    created_at: 30,
                }],
            },
            json!({
                "runs": [{
                    "id": "run-1",
                    "name": "Workflow workflow-1 30",
                    "workspaceId": "workspace-1",
                    "projectId": "project-1",
                    "workflowId": "workflow-1",
                    "version": "v30",
                    "status": "pending",
                    "startedAt": null,
                    "finishedAt": null,
                    "createdAt": 30,
                }],
            }),
        );
        assert_serialized_json(
            &ListWorkflowRunsByWorkflowRequest {
                workflow_id: "workflow-1".to_string(),
            },
            json!({ "workflowId": "workflow-1" }),
        );
        assert_serialized_json(
            &ListWorkflowRunsByWorkflowResponse {
                runs: vec![WorkflowRunSummary {
                    id: "run-1".to_string(),
                    name: "Workflow workflow-1 30".to_string(),
                    workspace_id: "workspace-1".to_string(),
                    project_id: "project-1".to_string(),
                    workflow_id: "workflow-1".to_string(),
                    version: "v30".to_string(),
                    status: WorkflowRunStatus::Pending,
                    started_at: None,
                    finished_at: None,
                    created_at: 30,
                }],
            },
            json!({
                "runs": [{
                    "id": "run-1",
                    "name": "Workflow workflow-1 30",
                    "workspaceId": "workspace-1",
                    "projectId": "project-1",
                    "workflowId": "workflow-1",
                    "version": "v30",
                    "status": "pending",
                    "startedAt": null,
                    "finishedAt": null,
                    "createdAt": 30,
                }],
            }),
        );
        assert_serialized_json(
            &ListWorkflowNodeRunsRequest {
                run_id: "run-1".to_string(),
            },
            json!({ "runId": "run-1" }),
        );
        assert_serialized_json(
            &ListWorkflowNodeRunsResponse { nodes: vec![node] },
            json!({
                "nodes": [{
                    "id": "node-1",
                    "runId": "run-1",
                    "scopeId": "root:run-1",
                    "nodeId": "start",
                    "nodeType": "start",
                    "sessionId": null,
                    "status": "succeeded",
                    "input": null,
                    "output": null,
                    "error": null,
                    "payload": null,
                    "iteration": null,
                    "startedAt": 30,
                    "finishedAt": 31,
                    "createdAt": 30,
                    "updatedAt": 31,
                }],
            }),
        );
        assert_serialized_json(
            &DeleteWorkflowRunRequest {
                run_id: "run-1".to_string(),
            },
            json!({ "runId": "run-1" }),
        );
        assert_serialized_json(
            &DeleteWorkflowRunResponse {
                run_id: "run-1".to_string(),
            },
            json!({ "runId": "run-1" }),
        );
        assert_serialized_json(
            &CompleteWorkflowNodeRequest {
                run_id: "run-1".to_string(),
                node_id: "node-1".to_string(),
                requester: None,
            },
            json!({ "runId": "run-1", "nodeId": "node-1" }),
        );
        assert_serialized_json(
            &CompleteWorkflowNodeRequest {
                run_id: "run-1".to_string(),
                node_id: "node-1".to_string(),
                requester: Some(NodeCompletionRequester::Human),
            },
            json!({ "runId": "run-1", "nodeId": "node-1", "requester": "human" }),
        );
        assert_serialized_json(
            &CompleteWorkflowNodeResponse { run },
            json!({
                "run": {
                    "id": "run-1",
                    "workspaceId": "workspace-1",
                    "workflowId": "workflow-1",
                    "snapshotId": "snapshot-1",
                    "name": "Workflow workflow-1 30",
                    "status": "pending",
                    "state": "{\"current_nodes\":[]}",
                    "input": "kickoff",
                    "output": null,
                    "error": null,
                    "startedAt": null,
                    "finishedAt": null,
                    "createdAt": 30,
                    "updatedAt": 30,
                }
            }),
        );
        assert_serialized_json(&WorkflowRunStatus::AwaitingInput, json!("awaitingInput"));
    }

    /// Omitting the run-level switch deserializes as `None` (handlers treat that as on).
    #[test]
    fn create_workflow_run_request_defaults_inject_last_failure_to_none() {
        let omitted: CreateWorkflowRunRequest =
            serde_json::from_str(r#"{"workspaceId":"w","workflowId":"f","locale":"zh-CN"}"#)
                .unwrap();
        assert_eq!(omitted.inject_last_failure, None);
        let off: CreateWorkflowRunRequest = serde_json::from_str(
            r#"{"workspaceId":"w","workflowId":"f","locale":"zh-CN","injectLastFailure":false}"#,
        )
        .unwrap();
        assert_eq!(off.inject_last_failure, Some(false));
    }

    /// A resume request without `rollback` stays `Keep`; snake_case values map onto the enum.
    #[test]
    fn deserializes_resume_workflow_run_request_rollback() {
        let omitted: ResumeWorkflowRunRequest =
            serde_json::from_value(json!({ "runId": "r" })).unwrap();
        assert_eq!(
            omitted,
            ResumeWorkflowRunRequest {
                run_id: "r".to_string(),
                rollback: None,
                snapshot_id: None,
            }
        );
        let node_files: ResumeWorkflowRunRequest =
            serde_json::from_value(json!({ "runId": "r", "rollback": "node_files" })).unwrap();
        assert_eq!(
            node_files,
            ResumeWorkflowRunRequest {
                run_id: "r".to_string(),
                rollback: Some(ResumeRollbackMode::NodeFiles),
                snapshot_id: None,
            }
        );
    }

    /// Omitting `snapshotId` deserializes as `None` so keep-resume stays the default.
    #[test]
    fn deserializes_resume_workflow_run_request_without_snapshot_id() {
        let omitted: ResumeWorkflowRunRequest =
            serde_json::from_value(json!({ "runId": "r" })).unwrap();
        assert_eq!(omitted.snapshot_id, None);
    }

    /// Diagnosis request identifiers stay camelCase on the wire.
    #[test]
    fn diagnose_workflow_node_failure_request_round_trips() {
        let request = DiagnoseWorkflowNodeFailureRequest {
            run_id: "run-1".to_string(),
            node_id: "agent".to_string(),
        };
        assert_serialized_json(&request, json!({ "runId": "run-1", "nodeId": "agent" }));
        let parsed: DiagnoseWorkflowNodeFailureRequest =
            serde_json::from_value(json!({ "runId": "run-1", "nodeId": "agent" })).unwrap();
        assert_eq!(parsed, request);
    }

    /// Diagnosis payload field names stay camelCase so the inspector can render the guess as stored.
    #[test]
    fn workflow_node_ai_diagnosis_round_trips() {
        let diagnosis = WorkflowNodeAiDiagnosis {
            text: "the schema rejected the reply".to_string(),
            agent_cli: "open_code".to_string(),
            model: "m".to_string(),
            generated_at: 50,
        };
        assert_serialized_json(
            &diagnosis,
            json!({
                "text": "the schema rejected the reply",
                "agentCli": "open_code",
                "model": "m",
                "generatedAt": 50,
            }),
        );
        let parsed: WorkflowNodeAiDiagnosis = serde_json::from_value(json!({
            "text": "the schema rejected the reply",
            "agentCli": "open_code",
            "model": "m",
            "generatedAt": 50,
        }))
        .unwrap();
        assert_eq!(parsed, diagnosis);
    }

    /// Serializes one value and compares the full JSON payload so field names stay stable.
    fn assert_serialized_json(value: &impl Serialize, expected: Value) {
        assert_eq!(serde_json::to_value(value).unwrap(), expected);
    }
}
