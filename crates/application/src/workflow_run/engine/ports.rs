use super::skill_delivery::SkillMaterializationReceipt;
use crate::RepositoryError;
use crate::workflow_run::engine::graph::WorkflowGraph;
use ora_domain::{
    SessionId, WorkflowNodeRun, WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRun, WorkflowRunId,
    Workspace,
};
use std::path::Path;
use thiserror::Error;

/// A node-run the engine wants to start in one scheduling wave.
///
/// The engine assigns the node-run id; the repository persists the row and the `current_nodes`
/// anchor in the same transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeRunToStart {
    pub id: WorkflowNodeRunId,
    pub node_id: String,
    pub node_type: String,
    pub input: Option<String>,
}

/// One file's incremental change made by a node execution, captured from the worktree git diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// Worktree-relative file path.
    pub path: String,
    /// Lines added by this node.
    pub additions: u64,
    /// Lines removed by this node.
    pub deletions: u64,
}

/// Everything the engine needs to start or drive a run, fetched in one read.
///
/// `graph_json` is the raw frozen React Flow document; the engine parses it into a `WorkflowGraph`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionContext {
    pub run: WorkflowRun,
    pub workspace: Workspace,
    pub graph_json: String,
}

/// Supplies new node-run identifiers for the engine's scheduling waves.
pub trait WorkflowNodeRunIdGenerator {
    /// Produces the identifier for a newly created node run.
    fn generate_node_run_id(&self) -> WorkflowNodeRunId;
}

/// Failures raised while setting up a run workspace's initial state at deploy time.
#[derive(Debug, Error)]
pub enum StartPrerequisitesError {
    #[error("workflow skill not found: {skill_id}")]
    WorkflowSkillNotFound { skill_id: String },
    #[error("workflow role not found: {role_id}")]
    WorkflowRoleNotFound { role_id: String },
    #[error("skill materialization failed: {message}")]
    SkillMaterializationError { message: String },
    #[error("agent {agent_ref} does not support workflow-managed skills")]
    AgentSkillDeliveryUnsupported { agent_ref: String },
    #[error("failed to resolve skill delivery for agent {agent_ref}: {message}")]
    AgentSkillDeliveryError { agent_ref: String, message: String },
    #[error("repository operation failed")]
    Repository(#[from] RepositoryError),
}

/// Validates and materializes a run workspace's initial state at deploy time.
///
/// Skills and roles are deploy dependencies: every agent's role must resolve in the agents catalog
/// and every enabled skill must resolve in the catalog. The backend implementation also copies the
/// enabled skills into the worktree-relative discovery roots declared by each Agent's delivery
/// capability while the worktree is being created, so the run's initial state is complete before
/// it is persisted and `start` needs no re-validation.
pub trait WorkflowRunWorkspaceInitializer: Send + Sync {
    /// Resolves every declared role and skill in the graph and materializes the enabled skills
    /// into the selected run workspace.
    fn initialize_workspace(
        &self,
        graph: &WorkflowGraph,
        workspace_root: &Path,
    ) -> Result<SkillMaterializationReceipt, StartPrerequisitesError>;
}

/// Outcome of starting a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartWorkflowRunResult {
    /// The run transitioned from `Pending` (empty `current_nodes`) to `Running`.
    Started,
    /// The run is not startable; the caller returns the current run idempotently.
    Current,
    NotFound,
}

/// Outcome of advancing one node-run (`complete` or `fail`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdvanceWorkflowRunResult {
    /// The node-run transitioned and the run state was maintained.
    Advanced,
    /// The node-run is not `Running` (a late or duplicate callback); the transition is a no-op.
    NotRunning,
    NotFound,
}

/// Outcome of cancelling a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelWorkflowRunResult {
    Cancelled,
    NotActive,
    NotFound,
}

/// Outcome of restarting a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartWorkflowRunResult {
    Restarted,
    NotRestartable,
    NotFound,
}

/// Outcome of publishing a prepared workflow node session to observers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindWorkflowNodeSessionResult {
    /// The run and node are still running, and the session is now visible to observers.
    Bound,
    /// Cancellation or another terminal transition won before the session could be published.
    NotRunning,
    /// The node or its owning run no longer exists.
    NotFound,
}

/// Outcome of updating a run's kickoff input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateWorkflowRunInputResult {
    Updated,
    /// The run is executing (`Running`, or a `Pending` pause with in-flight nodes), so its
    /// input is frozen. A not-started `Pending` run or any terminal run is editable.
    NotEditable,
    NotFound,
}

/// Persistence operations for the workflow run execution engine.
///
/// This port is deliberately separate from the graph-agnostic `WorkflowRunRepository` CRUD port:
/// the engine owns node-run writes and the run state machine, and every state transition must be
/// a single immediate transaction that maintains `state.current_nodes`. No generic overwrite of
/// the full run state is exposed to callers.
pub trait WorkflowRunEngineRepository {
    /// Loads the run, its workspace, and the frozen snapshot graph in one read.
    fn find_execution_context(
        &self,
        run_id: &WorkflowRunId,
    ) -> Result<Option<ExecutionContext>, RepositoryError>;

    /// Lists the node-run rows of one run so the engine can recompute ready and in-flight sets.
    fn list_node_runs(
        &self,
        run_id: &WorkflowRunId,
    ) -> Result<Vec<WorkflowNodeRun>, RepositoryError>;

    /// Publishes a node's prepared Ora session only while both the node and run are still running.
    ///
    /// The executor calls this after the initial prompt is accepted. Keeping `session_id` absent
    /// until then prevents a workflow transcript load from displacing that owning prompt, while
    /// the guarded result lets a cancellation that won the race trigger immediate session cleanup.
    fn bind_node_run_session(
        &self,
        node_run_id: &WorkflowNodeRunId,
        session_id: &SessionId,
        now: i64,
    ) -> Result<BindWorkflowNodeSessionResult, RepositoryError>;

    /// Finds the live node run bound to a session, if any.
    ///
    /// Session bindings are per-node (each node warms its own session), so at most one node run
    /// carries a given session id; the interactive prompt hook uses this to flip the owning node
    /// between `Pending` and `Running`.
    fn find_node_run_by_session_id(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<WorkflowNodeRun>, RepositoryError>;

    /// Finds one node run by id, if it exists and is not soft-deleted.
    ///
    /// Used by baseline cleanup to decide whether a node's side file is still needed: a baseline
    /// survives only while its node is still awaiting input.
    fn find_node_run_by_id(
        &self,
        node_run_id: &WorkflowNodeRunId,
    ) -> Result<Option<WorkflowNodeRun>, RepositoryError>;

    /// Transitions one node run's status only when its current status is exactly `from`.
    ///
    /// Awaiting interactive nodes flip between `Pending` and `Running` around a human turn; the
    /// guard makes a stale flip against a completed or cancelled node a no-op (`NotRunning`).
    fn transition_node_run_status(
        &self,
        node_run_id: &WorkflowNodeRunId,
        from: WorkflowNodeStatus,
        to: WorkflowNodeStatus,
        now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError>;

    /// Starts a run by creating the start node-run and transitioning the run to `Running`.
    ///
    /// Only a `Pending` run with empty `current_nodes` transitions; anything else returns
    /// `Current` so callers can return the existing run idempotently.
    fn start_run(
        &self,
        run_id: &WorkflowRunId,
        start_node_run: &NodeRunToStart,
        now: i64,
    ) -> Result<StartWorkflowRunResult, RepositoryError>;

    /// Starts a wave of ready nodes, creating their node-run rows and updating `current_nodes`.
    fn start_ready_nodes(
        &self,
        run_id: &WorkflowRunId,
        node_runs: &[NodeRunToStart],
        now: i64,
    ) -> Result<(), RepositoryError>;

    /// Marks one node-run succeeded, records its final assistant output, stop reason, and file
    /// changes, and removes it from `current_nodes`.
    fn complete_node(
        &self,
        node_run_id: &WorkflowNodeRunId,
        output: Option<String>,
        stop_reason: Option<String>,
        file_changes: Vec<FileChange>,
        now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError>;

    /// Marks one node-run and its run failed, anchoring the failed node in `current_nodes`.
    fn fail_node(
        &self,
        node_run_id: &WorkflowNodeRunId,
        error: String,
        output: Option<String>,
        now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError>;

    /// Finishes a run as succeeded with the given output.
    fn finish_run(
        &self,
        run_id: &WorkflowRunId,
        output: Option<String>,
        now: i64,
    ) -> Result<(), RepositoryError>;

    /// Cancels a running run: the run and its non-terminal node runs become `Cancelled` and
    /// `current_nodes` is cleared.
    fn cancel_run(
        &self,
        run_id: &WorkflowRunId,
        now: i64,
    ) -> Result<CancelWorkflowRunResult, RepositoryError>;

    /// Restarts a non-running run: soft-deletes its node runs and resets it to `Pending` with
    /// empty `current_nodes`, so prior node-run history stays queryable.
    fn restart_run(
        &self,
        run_id: &WorkflowRunId,
        now: i64,
    ) -> Result<RestartWorkflowRunResult, RepositoryError>;

    /// Sets the kickoff input of a `Pending` run with empty `current_nodes`, so the start node
    /// receives it when the run starts.
    fn update_run_input(
        &self,
        run_id: &WorkflowRunId,
        input: Option<String>,
        now: i64,
    ) -> Result<UpdateWorkflowRunInputResult, RepositoryError>;

    /// Lists runs in `Running` or `Failed` status for boot-time crash recovery.
    fn list_recoverable_runs(&self) -> Result<Vec<WorkflowRunId>, RepositoryError>;

    /// Fails the non-terminal node runs of the given runs and stops their running sessions.
    ///
    /// A run whose in-flight nodes are all `Pending` (awaiting interactive input) is preserved
    /// as-is: it was parked on the human rather than computing, so a restart must not destroy it.
    fn fail_orphaned_node_runs(
        &self,
        run_ids: &[WorkflowRunId],
        now: i64,
    ) -> Result<(), RepositoryError>;
}
