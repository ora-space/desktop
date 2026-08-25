use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Describes the lifecycle state of a workflow run in the public contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflowRun.ts")]
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
#[ts(export_to = "workflowRun.ts")]
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
#[ts(export_to = "workflowRun.ts")]
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
    pub payload: Option<String>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Public node-run payload without persistence audit metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflowRun.ts")]
pub struct WorkflowNodeRun {
    pub id: String,
    pub run_id: String,
    pub node_id: String,
    pub node_type: String,
    pub session_id: Option<String>,
    pub status: WorkflowNodeStatus,
    pub input: Option<String>,
    pub output: Option<String>,
    pub error: Option<String>,
    pub payload: Option<String>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Lightweight run summary for list views with direct workspace ownership.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflowRun.ts")]
pub struct WorkflowRunSummary {
    pub id: String,
    pub name: String,
    pub workspace_id: String,
    pub project_id: String,
    pub workflow_id: String,
    pub status: WorkflowRunStatus,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub created_at: i64,
}

/// Identifies the Ora display language frozen for generated workflow-run prompts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "workflowRun.ts")]
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
#[ts(export_to = "workflowRun.ts")]
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
}

/// Returns the created workspace-owned run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflowRun.ts")]
pub struct CreateWorkflowRunResponse {
    pub run: WorkflowRun,
}

// ── Get by ID ──

/// Identifies the workflow run to retrieve by its stable identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflowRun.ts")]
pub struct GetWorkflowRunRequest {
    pub run_id: String,
}

/// Returns the full run detail including its display name and node runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflowRun.ts")]
pub struct GetWorkflowRunResponse {
    pub run: WorkflowRun,
    pub name: String,
    pub workspace_id: String,
    pub project_id: String,
    pub nodes: Vec<WorkflowNodeRun>,
}

// ── List by project ──

/// Requests the workflow run summaries for one project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflowRun.ts")]
pub struct ListWorkflowRunsRequest {
    pub project_id: String,
}

/// Returns the visible run summaries for the project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflowRun.ts")]
pub struct ListWorkflowRunsResponse {
    pub runs: Vec<WorkflowRunSummary>,
}

// ── List by workflow ──

/// Requests the workflow run summaries for one workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflowRun.ts")]
pub struct ListWorkflowRunsByWorkflowRequest {
    pub workflow_id: String,
}

/// Returns the visible run summaries for the workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflowRun.ts")]
pub struct ListWorkflowRunsByWorkflowResponse {
    pub runs: Vec<WorkflowRunSummary>,
}

// ── List node runs ──

/// Identifies the run whose node-run history to retrieve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflowRun.ts")]
pub struct ListWorkflowNodeRunsRequest {
    pub run_id: String,
}

/// Returns the node-run records of one run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflowRun.ts")]
pub struct ListWorkflowNodeRunsResponse {
    pub nodes: Vec<WorkflowNodeRun>,
}

// ── Delete ──

/// Identifies the workflow run to soft-delete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflowRun.ts")]
pub struct DeleteWorkflowRunRequest {
    pub run_id: String,
}

/// Returns the identifier of the soft-deleted run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflowRun.ts")]
pub struct DeleteWorkflowRunResponse {
    pub run_id: String,
}

/// Identifies the workflow run whose Workspace-owned display name should change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflowRun.ts")]
pub struct RenameWorkflowRunRequest {
    pub run_id: String,
    pub name: String,
}

/// Returns the workflow run after its display name was replaced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflowRun.ts")]
pub struct RenameWorkflowRunResponse {
    pub run: WorkflowRun,
}

// ── Start / Cancel / Restart (execution engine) ──

/// Identifies the run to start executing against its frozen snapshot graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflowRun.ts")]
pub struct StartWorkflowRunRequest {
    pub run_id: String,
}

/// Returns the run after starting (or idempotently its current state).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflowRun.ts")]
pub struct StartWorkflowRunResponse {
    pub run: WorkflowRun,
}

/// Identifies the running run whose node sessions should be stopped and run cancelled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflowRun.ts")]
pub struct CancelWorkflowRunRequest {
    pub run_id: String,
}

/// Returns the cancelled run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflowRun.ts")]
pub struct CancelWorkflowRunResponse {
    pub run: WorkflowRun,
}

/// Identifies the non-running run to reset and re-run from its start node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflowRun.ts")]
pub struct RestartWorkflowRunRequest {
    pub run_id: String,
}

/// Returns the reset and re-running run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflowRun.ts")]
pub struct RestartWorkflowRunResponse {
    pub run: WorkflowRun,
}

/// Sets the kickoff input of a pending run, used as the start node's input on start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflowRun.ts")]
pub struct UpdateWorkflowRunInputRequest {
    pub run_id: String,
    pub input: Option<String>,
}

/// Returns the run with its updated input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflowRun.ts")]
pub struct UpdateWorkflowRunInputResponse {
    pub run: WorkflowRun,
}

// ── Complete workflow node (interactive fallback) ──

/// Who requested the completion of one workflow node.
///
/// Phase 1 carries only the human path; the agent/CLI path reuses the same command later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflowRun.ts")]
pub enum NodeCompletionRequester {
    Human,
}

/// Identifies the awaiting interactive node to complete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflowRun.ts")]
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
#[ts(export_to = "workflowRun.ts")]
pub struct CompleteWorkflowNodeResponse {
    pub run: WorkflowRun,
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    WorkflowRunStatus::export(config)?;
    WorkflowNodeStatus::export(config)?;
    WorkflowRun::export(config)?;
    WorkflowNodeRun::export(config)?;
    WorkflowRunSummary::export(config)?;
    WorkflowRunLocale::export(config)?;
    CreateWorkflowRunRequest::export(config)?;
    CreateWorkflowRunResponse::export(config)?;
    GetWorkflowRunRequest::export(config)?;
    GetWorkflowRunResponse::export(config)?;
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
    UpdateWorkflowRunInputRequest::export(config)?;
    UpdateWorkflowRunInputResponse::export(config)?;
    NodeCompletionRequester::export(config)?;
    CompleteWorkflowNodeRequest::export(config)?;
    CompleteWorkflowNodeResponse::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        CompleteWorkflowNodeRequest, CompleteWorkflowNodeResponse, CreateWorkflowRunRequest,
        CreateWorkflowRunResponse, DeleteWorkflowRunRequest, DeleteWorkflowRunResponse,
        GetWorkflowRunRequest, GetWorkflowRunResponse, ListWorkflowNodeRunsRequest,
        ListWorkflowNodeRunsResponse, ListWorkflowRunsByWorkflowRequest,
        ListWorkflowRunsByWorkflowResponse, ListWorkflowRunsRequest, ListWorkflowRunsResponse,
        NodeCompletionRequester, WorkflowNodeRun, WorkflowNodeStatus, WorkflowRun,
        WorkflowRunLocale, WorkflowRunStatus, WorkflowRunSummary,
    };
    use pretty_assertions::assert_eq;
    use serde::Serialize;
    use serde_json::{Value, json};

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
            payload: None,
            started_at: None,
            finished_at: None,
            created_at: 30,
            updated_at: 30,
        };
        let node = WorkflowNodeRun {
            id: "node-1".to_string(),
            run_id: "run-1".to_string(),
            node_id: "start".to_string(),
            node_type: "start".to_string(),
            session_id: None,
            status: WorkflowNodeStatus::Succeeded,
            input: None,
            output: None,
            error: None,
            payload: None,
            started_at: Some(30),
            finished_at: Some(31),
            created_at: 30,
            updated_at: 31,
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
                "payload": null,
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
                    "payload": null,
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
                    "payload": null,
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
                    "nodeId": "start",
                    "nodeType": "start",
                    "sessionId": null,
                    "status": "succeeded",
                    "input": null,
                    "output": null,
                    "error": null,
                    "payload": null,
                    "startedAt": 30,
                    "finishedAt": 31,
                    "createdAt": 30,
                    "updatedAt": 31,
                }],
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
                    "nodeId": "start",
                    "nodeType": "start",
                    "sessionId": null,
                    "status": "succeeded",
                    "input": null,
                    "output": null,
                    "error": null,
                    "payload": null,
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
                    "payload": null,
                    "startedAt": null,
                    "finishedAt": null,
                    "createdAt": 30,
                    "updatedAt": 30,
                }
            }),
        );
        assert_serialized_json(&WorkflowRunStatus::AwaitingInput, json!("awaitingInput"));
    }

    /// Serializes one value and compares the full JSON payload so field names stay stable.
    fn assert_serialized_json(value: &impl Serialize, expected: Value) {
        assert_eq!(serde_json::to_value(value).unwrap(), expected);
    }
}
