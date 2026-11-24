use super::failure::NodeFailure;
use super::iteration::RoundOutcome;
use super::retry::WorkflowRetryRepository;
use super::skill_delivery::SkillMaterializationReceipt;
use crate::RepositoryError;
use crate::workflow_run::engine::graph::WorkflowGraph;
use ora_domain::{
    SessionId, WorkflowExecutionScope, WorkflowNodeRun, WorkflowNodeRunId, WorkflowNodeStatus,
    WorkflowRun, WorkflowRunId, WorkflowScopeId, WorkflowSnapshotId, Workspace,
};
use std::collections::BTreeMap;
use std::path::Path;
use thiserror::Error;

/// A node-run the engine wants to start in one scheduling wave.
///
/// The engine assigns the node-run id; the repository persists the row and the `current_nodes`
/// anchor in the same transaction. Rows carrying an `iteration` belong to a composite region
/// round and never enter the outer `current_nodes` anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeRunToStart {
    pub id: WorkflowNodeRunId,
    pub scope_id: WorkflowScopeId,
    pub node_id: String,
    pub node_type: String,
    pub input: Option<String>,
    /// Composite-region round this row executes in; `None` for outer rows.
    pub iteration: Option<u32>,
}

/// How one node-run's failure propagates to its run (ADR "iteration composite runtime" D6).
///
/// The engine resolves the policy structurally — a failure inside a `continue`-strategy
/// composite region is absorbed by the runtime's ledger instead of failing the run — so the
/// scheduling core never branches on node types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailurePropagation {
    /// The row fails and the run fails in the same transaction (the existing behavior).
    Run,
    /// The row fails but the run stays active; the owning composite runtime records the round
    /// as failed and advances.
    Composite,
}

/// How one settled iteration round continues, committed in the same transaction as the ledger
/// entry so the round's terminal fact and the following transition never split.
///
/// This is the repository-facing form: the engine has already materialized node-run ids and
/// inputs for the rows a `StartNextRound` begins.
#[derive(Debug, Clone, PartialEq)]
pub enum IterationRoundContinuation {
    /// Start the next round: bind its `item`/`index` and start its first ready members.
    StartNextRound {
        round: u32,
        item: serde_json::Value,
        node_runs: Vec<NodeRunToStart>,
    },
    /// Complete the composite node, writing the ledger-derived exposed variables.
    Complete {
        exposed: Vec<(String, serde_json::Value)>,
        output: Option<String>,
    },
    /// Fail the composite node (and its run) — used when a `fail`-strategy round fails.
    Fail { error: String },
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
    pub root_scope_id: WorkflowScopeId,
    pub workspace: Workspace,
    pub graph_json: String,
}

/// Initial durable facts for one newly-created Loop round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopRoundToStart {
    pub id: WorkflowScopeId,
    pub parent_loop_node_run_id: WorkflowNodeRunId,
    pub round_index: u32,
    pub state: String,
    pub start_node_run: NodeRunToStart,
}

/// An atomic round transition, including an exit request while siblings are still active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopRoundAdvance {
    /// Freezes exit outputs and cancels remaining child rows before any session cleanup.
    RequestExit {
        node_run_id: WorkflowNodeRunId,
        result: Result<BTreeMap<String, serde_json::Value>, String>,
    },
    /// Closes the current round and starts the next round from its Start node.
    Continue { next: LoopRoundToStart },
    /// Closes the round and publishes the configured values through the parent Loop node.
    Succeed {
        outputs: BTreeMap<String, serde_json::Value>,
        reason: super::loop_round::LoopCompletionReason,
    },
    /// Terminates the current round, parent Loop, and root run with one durable error.
    Fail { error: String },
}

/// Supplies new node-run identifiers for the engine's scheduling waves.
pub trait WorkflowNodeRunIdGenerator {
    /// Produces the identifier for a newly created node run.
    fn generate_node_run_id(&self) -> WorkflowNodeRunId;

    /// Produces a distinct identity for a newly-created execution scope.
    fn generate_scope_id(&self) -> WorkflowScopeId {
        WorkflowScopeId::new(format!("scope:{}", self.generate_node_run_id()))
    }
}

/// Publishes run invalidation events after run or node-run state transitions commit
/// (ADR "node runtime orchestration" D7).
///
/// Events carry no workflow state: they only signal that one run's persisted state changed so
/// observers re-query the authoritative persistence. Publishing must be non-blocking, and a
/// lost or reordered event may only leave a stale view that the next event or refresh clears;
/// the persisted state remains the single source of truth.
pub trait WorkflowRunInvalidationPublisher: Send + Sync {
    /// Signals that the persisted state of one run changed; observers should re-query it.
    fn publish_run_invalidated(&self, run_id: &WorkflowRunId);
}

/// A publisher that drops every invalidation, for engines assembled without an event bus.
pub struct NoRunInvalidations;

impl WorkflowRunInvalidationPublisher for NoRunInvalidations {
    fn publish_run_invalidated(&self, _run_id: &WorkflowRunId) {}
}

/// Failures raised while setting up a run workspace's initial state at deploy time.
///
/// Every dependency failure names both the node that declared the dependency and the dependency
/// itself, because the run cannot start and the operator has to know what to restore.
#[derive(Debug, Error)]
pub enum StartPrerequisitesError {
    #[error("workflow skill not found for node {node_id}: {skill_id}")]
    WorkflowSkillNotFound {
        node_id: String,
        node_title: String,
        skill_id: String,
    },
    #[error("workflow MCP not installed for node {node_id}: {mcp_id}")]
    WorkflowMcpNotFound {
        node_id: String,
        node_title: String,
        mcp_id: String,
    },
    #[error("workflow role not found: {role_id}")]
    WorkflowRoleNotFound { role_id: String },
    #[error("skill materialization failed: {message}")]
    SkillMaterializationError { message: String },
    #[error("repository operation failed")]
    Repository(#[from] RepositoryError),
}

/// Validates a run workspace's roles and Effect-owned skill placements at deploy time.
///
/// Skills and roles are deploy dependencies: every agent's role must resolve in the agents catalog
/// and every enabled skill must resolve in the catalog. Physical skill materialization is owned by
/// the Effect subsystem; this port freezes the paths that the workflow prompt will reference.
pub trait WorkflowRunWorkspaceInitializer: Send + Sync {
    /// Resolves every declared role and skill and freezes each enabled skill's discovery paths.
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

/// Outcome of resuming a failed or cancelled run from its failed nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeWorkflowRunResult {
    Resumed,
    NotResumable,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateWorkflowRunInputResult {
    Updated,
    /// The run is executing (`Running`, or a `Pending` pause with in-flight nodes), so its
    /// input is frozen. A not-started `Pending` run or any terminal run is editable.
    NotEditable,
    NotFound,
    /// One supplied value was rejected before entering the pool; the run keeps its previously
    /// stored input. Carries which Start variable was rejected and why, so the run-input screen
    /// can point the user at the exact field instead of a generic failure.
    ///
    /// Single-rejection contract: a submission reports at most one rejection — the first
    /// refused value in the payload's alphabetical fold order — and no value from that
    /// submission is applied.
    Rejected(RunInputRejection),
}

/// One Start variable the run-input boundary refused, with the rejection's cause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunInputRejection {
    /// The Start variable's declared name, the identifier the run-input payload keys values by.
    /// The run-input screen may label the field with a configured display name instead, so
    /// clients highlighting the rejected field must match on this name, not the screen label.
    pub variable: String,
    pub reason: RunInputRejectionReason,
}

/// Why a supplied Start value cannot enter the run's variable pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunInputRejectionReason {
    /// The run's Start node declares no variable with this name.
    Undeclared,
    /// The declared variable is owned by a writer other than the run's Start node, so the
    /// run-input screen cannot set it.
    NotStartOwned,
    /// The value does not satisfy the variable's declared pool type.
    TypeMismatch { expected_type: String },
    /// The value is longer than the variable's declared maximum length.
    LengthExceeded { max_length: usize },
}

impl RunInputRejectionReason {
    /// Renders the stable English detail carried to the public error contract. The UI may show
    /// it verbatim next to a localized title, like plugin package rejection reasons.
    pub fn detail(&self) -> String {
        match self {
            Self::Undeclared => "no declared Start variable has this name".to_string(),
            Self::NotStartOwned => "variable is not owned by the run's Start node".to_string(),
            Self::TypeMismatch { expected_type } => {
                format!("value does not match the declared type {expected_type}")
            }
            Self::LengthExceeded { max_length } => {
                format!("value exceeds the maximum length {max_length}")
            }
        }
    }
}

/// Stable public detail for a Start variable whose required value is missing at start time.
///
/// Start-time validation failures reuse the run-input rejection's public error, so their
/// user-facing details belong beside the other stable reason strings instead of living as
/// inline literals in adapter mapping layers.
pub const MISSING_REQUIRED_START_DETAIL: &str = "required value is missing";

/// Stable public detail for a Start value outside its select options at start time.
pub const INVALID_START_OPTION_DETAIL: &str = "value is not one of the configured options";

/// Persistence operations for the workflow run execution engine.
///
/// This port is deliberately separate from the graph-agnostic `WorkflowRunRepository` CRUD port:
/// the engine owns node-run writes and the run state machine, and every state transition must be
/// a single immediate transaction that maintains `state.current_nodes`. No generic overwrite of
/// the full run state is exposed to callers.
///
/// Automatic retries persist through the [`WorkflowRetryRepository`] supertrait.
pub trait WorkflowRunEngineRepository: WorkflowRetryRepository {
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

    /// Returns the most recent soft-deleted `Failed` run of `(node_id, iteration)` inside `run_id`
    /// (the attempt that `resume_from_failure` cleared), or `None` when that pair never failed.
    fn find_last_failed_attempt(
        &self,
        run_id: &WorkflowRunId,
        node_id: &str,
        iteration: Option<u32>,
    ) -> Result<Option<WorkflowNodeRun>, RepositoryError>;

    /// Loads one Loop's active round, if it has already been created.
    fn find_active_loop_round(
        &self,
        parent_loop_node_run_id: &WorkflowNodeRunId,
    ) -> Result<Option<WorkflowExecutionScope>, RepositoryError>;

    /// Lists node instances belonging to one root or round scope.
    fn list_node_runs_in_scope(
        &self,
        scope_id: &WorkflowScopeId,
    ) -> Result<Vec<WorkflowNodeRun>, RepositoryError>;

    /// Publishes a node's prepared Ora session while the node run is still `Running`.
    ///
    /// Design rule D2: a run that is already `Failed` still accepts bindings for its in-flight
    /// nodes so they persist `Succeeded` or `Failed` on their own merits. Cancellation rejects
    /// through the node-run status, because `cancel_run` marks every non-terminal node
    /// `Cancelled` in the same transaction that cancels the run.
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

    /// Creates a Loop round and its child Start node atomically while the parent is active.
    fn start_loop_round(
        &self,
        run_id: &WorkflowRunId,
        round: &LoopRoundToStart,
        now: i64,
    ) -> Result<(), RepositoryError>;

    /// Starts child nodes without changing the root run's `current_nodes` anchor.
    fn start_scope_ready_nodes(
        &self,
        scope_id: &WorkflowScopeId,
        node_runs: &[NodeRunToStart],
        now: i64,
    ) -> Result<(), RepositoryError>;

    /// Closes a drained Loop round and advances its parent atomically.
    fn advance_loop_round(
        &self,
        scope_id: &WorkflowScopeId,
        advance: &LoopRoundAdvance,
        now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError>;

    /// Marks one node-run succeeded, records its final assistant output, stop reason, and file
    /// changes, and removes it from `current_nodes`.
    ///
    /// `structured_output` carries the parsed, schema-validated object of an agent node's
    /// structured-output contract, written to `{node}.structured_output` alongside the status
    /// transition in the same transaction. `payload_patch` is merged into the node payload in that
    /// same transaction, which is how a forced interactive completion records its audit without a
    /// second write that could be lost between the commit and the audit.
    #[allow(clippy::too_many_arguments)]
    fn complete_node(
        &self,
        node_run_id: &WorkflowNodeRunId,
        output: Option<String>,
        structured_output: Option<serde_json::Value>,
        stop_reason: Option<String>,
        file_changes: Vec<FileChange>,
        payload_patch: Option<&serde_json::Value>,
        now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError>;

    /// Marks one node-run failed with a classified `NodeFailure` and a propagation policy.
    ///
    /// Persists `failure.message` in `error`, `failure.output` in `output`, and the full
    /// `NodeFailureDetail` under `payload.error_detail`. `FailurePropagation::Run` keeps the
    /// existing behavior: the row and its run fail in the same transaction.
    /// `FailurePropagation::Composite` fails only the row; the run stays active so the owning
    /// composite runtime settles the failed round and advances.
    fn fail_node(
        &self,
        node_run_id: &WorkflowNodeRunId,
        failure: NodeFailure,
        propagation: FailurePropagation,
        now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError>;

    /// Starts one composite-region round atomically: binds the round's `item` and `index` pool
    /// variables and inserts the round's first node-run rows in one transaction, so a crash
    /// leaves either the whole round unstarted or the round variables consistent with its rows
    /// (ADR "iteration composite runtime" D2).
    ///
    /// Round rows never enter the outer `current_nodes` anchor; the owner stays anchored while
    /// running. Every row in `node_runs` must carry `iteration == Some(round)`.
    fn start_iteration_round(
        &self,
        run_id: &WorkflowRunId,
        owner_node_id: &str,
        round: u32,
        item: &serde_json::Value,
        node_runs: &[NodeRunToStart],
        now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError>;

    /// Settles one drained round and continues atomically: the ledger entry, the round's
    /// terminal fact, and the continuation (next round start, node completion, or node
    /// failure) commit in one transaction. Recording an already-settled round is a no-op so
    /// replanning after a crash stays idempotent.
    fn settle_iteration_round(
        &self,
        run_id: &WorkflowRunId,
        owner_node_id: &str,
        round: u32,
        entry: RoundOutcome,
        continuation: IterationRoundContinuation,
        now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError>;

    /// Completes one composite node-run, writing its ledger-derived exposed variables through
    /// the pool's typed `set` and the node's display `output` in one transaction. Used for the
    /// empty-iterator-source path, where no round ever settles.
    fn complete_iteration_node(
        &self,
        node_run_id: &WorkflowNodeRunId,
        owner_node_id: &str,
        exposed: &[(String, serde_json::Value)],
        output: Option<String>,
        now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError>;

    /// Records the git checkpoint taken before a node started (or why none could be taken) under
    /// `payload.checkpoint` / `payload.checkpoint_error`, and the run snapshot id under
    /// `payload.snapshot_id`. Provenance only: never fails the node.
    fn record_node_checkpoint(
        &self,
        node_run_id: &WorkflowNodeRunId,
        snapshot_id: &str,
        checkpoint: Option<&str>,
        checkpoint_error: Option<&str>,
        now: i64,
    ) -> Result<(), RepositoryError>;

    /// Persists the previous-failure block that was injected into this node's prompt, under
    /// `payload.injected_failure_context`. Provenance for the inspector: never fails the node.
    fn record_node_injected_failure(
        &self,
        node_run_id: &WorkflowNodeRunId,
        text: &str,
    ) -> Result<(), RepositoryError>;

    /// Merges `payload.ai_diagnosis` onto one node-run row, overwriting a previous guess.
    ///
    /// Provenance only: nothing in scheduling, resume, or rollback reads this key. A missing row
    /// is a no-op so a late write after resume cannot fail the request.
    fn record_node_ai_diagnosis(
        &self,
        node_run_id: &WorkflowNodeRunId,
        diagnosis_json: &str,
        now: i64,
    ) -> Result<(), RepositoryError>;

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

    /// Clears the given node runs (soft-delete) and their pool writes so the scheduler can
    /// re-dispatch them, and moves a `Failed`/`Cancelled` run back to `Running`.
    fn resume_from_failure(
        &self,
        run_id: &WorkflowRunId,
        node_ids_to_clear: &[String],
        now: i64,
    ) -> Result<ResumeWorkflowRunResult, RepositoryError>;

    /// Points a Failed/Cancelled run at another snapshot and stores the migrated payload in one
    /// transaction. `false` when the run is missing or not in a resumable status.
    fn switch_run_snapshot(
        &self,
        run_id: &WorkflowRunId,
        snapshot_id: &WorkflowSnapshotId,
        payload_json: &str,
        now: i64,
    ) -> Result<bool, RepositoryError>;

    /// Sets the kickoff input of a `Pending` run with empty `current_nodes`, so the start node
    /// receives it when the run starts.
    fn update_run_input(
        &self,
        run_id: &WorkflowRunId,
        input: Option<String>,
        variables: BTreeMap<String, serde_json::Value>,
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

    /// Fails the given interrupted node-run rows of one run as `interrupted_by_restart`, while
    /// preserving the run and its `Running` composite rows so their runtimes settle the
    /// interrupted rounds on the next advance (ADR "iteration composite runtime" D2: the
    /// sweep's all-rows-fail behavior is relaxed only for composite rows).
    ///
    /// Orphaned sessions of the run's workspace are stopped, matching the whole-run sweep.
    fn fail_interrupted_node_runs(
        &self,
        run_id: &WorkflowRunId,
        node_run_ids: &[WorkflowNodeRunId],
        now: i64,
    ) -> Result<(), RepositoryError>;
}
