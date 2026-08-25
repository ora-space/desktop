use crate::{
    AuditFields, DomainModelError, ProjectId, SessionId, WorkflowId, WorkflowNodeRunId,
    WorkflowRunId, WorkflowSnapshotId, WorkspaceId,
};
use serde::{Deserialize, Serialize};

/// Captures the lifecycle state of a workflow run without exposing database integer codes.
///
/// `Pending` covers both "not started" (`current_nodes=[]`) and a HITL pause
/// (`current_nodes=[waiting node]`); the terminal states freeze the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkflowRunStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl WorkflowRunStatus {
    /// Returns the integer code used by persistence adapters for this run status.
    pub fn database_value(self) -> i64 {
        match self {
            Self::Pending => 0,
            Self::Running => 1,
            Self::Succeeded => 2,
            Self::Failed => 3,
            Self::Cancelled => 4,
        }
    }

    /// Converts a persisted integer into a strongly typed run status.
    pub fn from_database_value(value: i64) -> Result<Self, DomainModelError> {
        match value {
            0 => Ok(Self::Pending),
            1 => Ok(Self::Running),
            2 => Ok(Self::Succeeded),
            3 => Ok(Self::Failed),
            4 => Ok(Self::Cancelled),
            _ => Err(DomainModelError::InvalidWorkflowRunStatus(value)),
        }
    }
}

impl TryFrom<i64> for WorkflowRunStatus {
    type Error = DomainModelError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        Self::from_database_value(value)
    }
}

/// Captures the lifecycle state of one node execution inside a run.
///
/// Nodes that have not started executing have no `WorkflowNodeRun` row; the frontend derives
/// "not started" by comparing graph nodes against the recorded node runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkflowNodeStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl WorkflowNodeStatus {
    /// Returns the integer code used by persistence adapters for this node status.
    pub fn database_value(self) -> i64 {
        match self {
            Self::Pending => 0,
            Self::Running => 1,
            Self::Succeeded => 2,
            Self::Failed => 3,
            Self::Cancelled => 4,
        }
    }

    /// Converts a persisted integer into a strongly typed node status.
    pub fn from_database_value(value: i64) -> Result<Self, DomainModelError> {
        match value {
            0 => Ok(Self::Pending),
            1 => Ok(Self::Running),
            2 => Ok(Self::Succeeded),
            3 => Ok(Self::Failed),
            4 => Ok(Self::Cancelled),
            _ => Err(DomainModelError::InvalidWorkflowNodeStatus(value)),
        }
    }
}

impl TryFrom<i64> for WorkflowNodeStatus {
    type Error = DomainModelError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        Self::from_database_value(value)
    }
}

/// Represents one execution of a published workflow snapshot inside a project.
///
/// The run pins `snapshot_id` to the user-released version and directly owns its workspace and
/// display name; no task row is needed to resolve execution context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub id: WorkflowRunId,
    pub workspace_id: WorkspaceId,
    pub workflow_id: WorkflowId,
    pub snapshot_id: WorkflowSnapshotId,
    pub name: String,
    pub status: WorkflowRunStatus,
    pub state: Option<String>,
    pub input: Option<String>,
    pub output: Option<String>,
    pub error: Option<String>,
    pub payload: Option<String>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub audit_fields: AuditFields,
}

impl WorkflowRun {
    /// Creates a workflow run with the workspace and display name that define its execution scope.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: WorkflowRunId,
        workspace_id: WorkspaceId,
        workflow_id: WorkflowId,
        snapshot_id: WorkflowSnapshotId,
        name: impl Into<String>,
        status: WorkflowRunStatus,
        state: Option<String>,
        input: Option<String>,
        output: Option<String>,
        error: Option<String>,
        payload: Option<String>,
        started_at: Option<i64>,
        finished_at: Option<i64>,
        audit_fields: AuditFields,
    ) -> Self {
        Self {
            id,
            workspace_id,
            workflow_id,
            snapshot_id,
            name: name.into(),
            status,
            state,
            input,
            output,
            error,
            payload,
            started_at,
            finished_at,
            audit_fields,
        }
    }
}

/// Represents one executed node inside a workflow run.
///
/// `node_id`/`node_type` come from the frozen snapshot graph; `session_id` is set only for
/// agent/prompt nodes that back a real Ora session. Node writes are owned by the execution
/// engine; the CRUD layer only reads them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowNodeRun {
    pub id: WorkflowNodeRunId,
    pub run_id: WorkflowRunId,
    pub node_id: String,
    pub node_type: String,
    pub session_id: Option<SessionId>,
    pub status: WorkflowNodeStatus,
    pub input: Option<String>,
    pub output: Option<String>,
    pub error: Option<String>,
    pub payload: Option<String>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub audit_fields: AuditFields,
}

impl WorkflowNodeRun {
    /// Creates a node-run snapshot together with its persistence-managed audit metadata.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: WorkflowNodeRunId,
        run_id: WorkflowRunId,
        node_id: impl Into<String>,
        node_type: impl Into<String>,
        session_id: Option<SessionId>,
        status: WorkflowNodeStatus,
        input: Option<String>,
        output: Option<String>,
        error: Option<String>,
        payload: Option<String>,
        started_at: Option<i64>,
        finished_at: Option<i64>,
        audit_fields: AuditFields,
    ) -> Self {
        Self {
            id,
            run_id,
            node_id: node_id.into(),
            node_type: node_type.into(),
            session_id,
            status,
            input,
            output,
            error,
            payload,
            started_at,
            finished_at,
            audit_fields,
        }
    }
}

/// Lightweight run summary for list views with direct workspace identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRunSummary {
    pub id: WorkflowRunId,
    pub name: String,
    pub workspace_id: WorkspaceId,
    pub project_id: ProjectId,
    pub workflow_id: WorkflowId,
    pub status: WorkflowRunStatus,
    /// Whether the run has an awaiting (`Pending`) interactive node; the wire status derives
    /// `AwaitingInput` from this while the persisted run status stays `Running`.
    pub has_awaiting_node: bool,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub created_at: i64,
}

/// Full run detail including the run record, its workspace context, and node runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowRunDetail {
    pub run: WorkflowRun,
    pub name: String,
    pub workspace_id: WorkspaceId,
    /// The project owning the workspace, mirroring the summary's `project_id`.
    pub project_id: ProjectId,
    pub nodes: Vec<WorkflowNodeRun>,
}
