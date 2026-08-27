use crate::RepositoryError;
use ora_domain::{
    ProjectId, WorkflowId, WorkflowNodeRun, WorkflowRun, WorkflowRunDetail, WorkflowRunId,
    WorkflowRunSummary, Workspace, WorkspaceId,
};

/// Reports whether run creation committed or lost workspace admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowRunCreateOutcome {
    Created(Box<WorkflowRun>),
    WorkspaceNotVisible,
}

/// Describes the outcome of soft-deleting a workflow run while preserving aggregate invariants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteWorkflowRunResult {
    Deleted,
    NotFound,
    ActiveRun,
}

/// Reads the workspace admission record needed before a workflow run is created.
///
/// Implementations expose the domain workspace, including its tagged location, so the creation
/// handler can validate that a run targets an existing execution environment without traversing a
/// Task or reconstructing a path from project metadata.
pub trait WorkspaceRepository {
    /// Loads one visible workspace by its stable identifier.
    fn find_workspace(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Option<Workspace>, RepositoryError>;

    /// Loads the canonical main workspace for a visible project.
    fn find_main_workspace(
        &self,
        project_id: &ProjectId,
    ) -> Result<Option<Workspace>, RepositoryError>;

    /// Reports whether the workspace provider has durably reached the ready state.
    fn is_provisioning_ready(&self, workspace_id: &WorkspaceId) -> Result<bool, RepositoryError>;
}

/// Defines graph-agnostic persistence operations for the workflow-run aggregate.
///
/// The execution engine computes graph-derived inputs and calls these methods; this layer never
/// parses the frozen snapshot graph.
pub trait WorkflowRunRepository {
    /// Persists a new run directly under its already selected workspace.
    ///
    /// Implementations re-validate that the workspace and project are visible and active in the
    /// same transaction as the insert, so a concurrently retiring workspace cannot admit a run.
    fn create_run(&self, run: WorkflowRun) -> Result<WorkflowRunCreateOutcome, RepositoryError>;

    /// Loads one visible run by identifier.
    fn find_run(&self, run_id: &WorkflowRunId) -> Result<Option<WorkflowRun>, RepositoryError>;

    /// Loads one visible run together with its workspace/project projection and node runs.
    fn get_run_detail(
        &self,
        run_id: &WorkflowRunId,
    ) -> Result<Option<WorkflowRunDetail>, RepositoryError>;

    /// Lists visible run summaries for a project, ordered by creation time.
    fn list_runs_by_project(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<WorkflowRunSummary>, RepositoryError>;

    /// Lists visible run summaries for a workflow, ordered by creation time.
    fn list_runs_by_workflow(
        &self,
        workflow_id: &WorkflowId,
    ) -> Result<Vec<WorkflowRunSummary>, RepositoryError>;

    /// Lists the node-run records of one run in stable ascending order.
    fn list_node_runs(
        &self,
        run_id: &WorkflowRunId,
    ) -> Result<Vec<WorkflowNodeRun>, RepositoryError>;

    /// Replaces a run's user-visible name without changing its execution state.
    fn rename_run(
        &self,
        run_id: &WorkflowRunId,
        name: String,
        updated_at: i64,
    ) -> Result<Option<WorkflowRun>, RepositoryError>;

    /// Soft-deletes one run, its node rows, and its node-owned sessions in a single transaction.
    ///
    /// A run is active when it is `Running`, has a non-terminal node run, or a `Running` session is
    /// bound to one of its node runs. A not-started `Pending` run (empty `current_nodes`, no node
    /// rows) is not active and can be discarded. Deleting a run never deletes its shared workspace.
    fn soft_delete_run(
        &self,
        run_id: &WorkflowRunId,
        deleted_at: i64,
    ) -> Result<DeleteWorkflowRunResult, RepositoryError>;
}

/// Supplies new workflow run identifiers for create use cases.
pub trait WorkflowRunIdGenerator {
    /// Produces the identifier for a newly created workflow run.
    fn generate_run_id(&self) -> WorkflowRunId;
}
