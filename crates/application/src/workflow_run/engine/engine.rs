use crate::RepositoryError;
use crate::project::Clock;
use crate::workflow_run::engine::branch_projection::BranchProjection;
use crate::workflow_run::engine::condition::ELSE_BRANCH_ID;
use crate::workflow_run::engine::graph::{GraphError, WorkflowGraph, WorkflowGraphNode};
use crate::workflow_run::engine::node_runtime::{
    CompositeAdvancePlan, CompositeContinuation, NodeRuntimeRegistry, RegisteredNodeRuntime,
    SwiftCompletion, standard_node_runtimes,
};
use crate::workflow_run::engine::node_type::NodeType;
use crate::workflow_run::engine::ports::{
    AdvanceWorkflowRunResult, CancelWorkflowRunResult, ExecutionContext, FailurePropagation,
    FileChange, IterationRoundContinuation, NoRunInvalidations, NodeRunToStart,
    RestartWorkflowRunResult, StartWorkflowRunResult, UpdateWorkflowRunInputResult,
    WorkflowNodeRunIdGenerator, WorkflowRunEngineRepository, WorkflowRunInvalidationPublisher,
};
use crate::workflow_run::engine::skill_delivery::WorkflowRunPayload;
use crate::workflow_run::engine::variable_pool::WorkflowVariablePool;
use ora_domain::{WorkflowNodeRun, WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRunId};
use std::collections::HashSet;
use std::sync::Arc;
use thiserror::Error;

/// Executes one agent node through a real session, calling the engine back when done.
///
/// The implementation lives in the backend and drives the session asynchronously; it MUST report
/// completion through `WorkflowRunEngine::complete_node`/`fail_node` on the same per-run serial
/// executor so state transitions stay serial. The engine wraps every `NodeExecutor` as the
/// Agent node runtime, so this port remains the backend's single integration seam.
pub trait NodeExecutor: Send + Sync {
    /// Dispatches one agent node; returns immediately while the session runs in the background.
    fn dispatch(
        &self,
        node_run_id: &WorkflowNodeRunId,
        node: &WorkflowGraphNode,
        context: &ExecutionContext,
    );
}

/// Reports node completion from the session driver back to the run engine.
///
/// The backend session driver invokes this when an agent node's session finishes; callbacks MUST
/// be routed through the run's serial executor so state transitions stay serial.
pub trait WorkflowRunCallback: Send + Sync {
    /// Reports a successful node completion with its final assistant output, stop reason, and
    /// incremental file changes.
    ///
    /// `structured_output` is the parsed, schema-validated object of a structured-output contract.
    fn complete_node(
        &self,
        run_id: &WorkflowRunId,
        node_run_id: &WorkflowNodeRunId,
        output: Option<String>,
        structured_output: Option<serde_json::Value>,
        stop_reason: Option<String>,
        file_changes: Vec<FileChange>,
    );

    /// Reports a failed node execution with an actionable error and any accumulated output.
    fn fail_node(
        &self,
        run_id: &WorkflowRunId,
        node_run_id: &WorkflowNodeRunId,
        error: String,
        output: Option<String>,
    );
}

/// Structural validation failures raised when starting a workflow run.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WorkflowValidationError {
    #[error("workflow graph has no start node")]
    MissingStartNode,
    #[error("node {node_id} has unsupported node type {node_type}")]
    UnsupportedNodeType {
        node_id: String,
        node_type: NodeType,
    },
    #[error("nodes are unreachable from the start node: {node_ids:?}")]
    UnreachableNodes { node_ids: Vec<String> },
    #[error("output node {node_id} has outgoing edges; output must be terminal")]
    OutputNodeHasSuccessors { node_id: String },
    #[error("condition node {node_id} declares case {case_id} more than once")]
    DuplicateConditionCase { node_id: String, case_id: String },
    #[error("condition node {node_id} has an edge on unknown branch {handle}")]
    UnknownConditionBranch { node_id: String, handle: String },
    #[error("output node {node_id} declares the result name {name} more than once")]
    DuplicateOutputName { node_id: String, name: String },
    #[error("required Start variable has no value: {name}")]
    MissingRequiredStartVariable { name: String },
    #[error("Start variable {name} is not one of its configured options")]
    InvalidStartVariableOption { name: String },
}

/// Failures surfaced by the workflow run engine.
#[derive(Debug, Error)]
pub enum EngineError {
    #[error("workflow run not found: {run_id}")]
    WorkflowRunNotFound { run_id: String },
    #[error("workflow graph is invalid")]
    GraphParse(#[from] GraphError),
    #[error("workflow graph is not executable")]
    Validation(#[from] WorkflowValidationError),
    #[error("workflow run repository operation failed")]
    Repository(#[from] RepositoryError),
}

/// Drives one workflow run through start/cancel/restart and the reactive DAG scheduler.
///
/// The engine is synchronous and stateless: every command recomputes the completed, in-flight,
/// and ready sets from persistence. Each node type's execution policy lives behind a
/// `NodeRuntime` registered in [`NodeRuntimeRegistry`]; the scheduling core dispatches on the
/// registered execution form, never on node types. The backend must route all commands and
/// callbacks for one run through a single serial executor.
#[derive(Clone)]
pub struct WorkflowRunEngine<R, G, C> {
    repository: R,
    runtimes: NodeRuntimeRegistry,
    node_run_id_generator: G,
    clock: C,
    run_events: Arc<dyn WorkflowRunInvalidationPublisher>,
}

impl<R, G, C> WorkflowRunEngine<R, G, C> {
    /// Builds an engine from its ports with the standard v1 node runtimes and no run
    /// invalidation events.
    pub fn new<E>(repository: R, agent_executor: E, node_run_id_generator: G, clock: C) -> Self
    where
        E: NodeExecutor + 'static,
    {
        Self::with_run_events(
            repository,
            agent_executor,
            node_run_id_generator,
            clock,
            Arc::new(NoRunInvalidations),
        )
    }

    /// Builds an engine that also publishes one run invalidation event after every run or
    /// node-run state transition commits (ADR "node runtime orchestration" D7).
    pub fn with_run_events<E>(
        repository: R,
        agent_executor: E,
        node_run_id_generator: G,
        clock: C,
        run_events: Arc<dyn WorkflowRunInvalidationPublisher>,
    ) -> Self
    where
        E: NodeExecutor + 'static,
    {
        Self {
            repository,
            runtimes: standard_node_runtimes(agent_executor),
            node_run_id_generator,
            clock,
            run_events,
        }
    }
}

impl<R, G, C> WorkflowRunEngine<R, G, C>
where
    R: WorkflowRunEngineRepository,
    G: WorkflowNodeRunIdGenerator,
    C: Clock,
{
    /// Starts a run after validating the frozen graph.
    ///
    /// Role and skill prerequisites are validated and materialized by the deploy flow when the run
    /// worktree is created, so `start` only validates graph executability before scheduling.
    pub fn start(&self, run_id: &WorkflowRunId) -> Result<StartWorkflowRunResult, EngineError> {
        let context = self.execution_context(run_id)?;
        let graph = WorkflowGraph::parse(&context.graph_json)?;
        let Some(start_node) = graph.start_node() else {
            return Err(WorkflowValidationError::MissingStartNode.into());
        };
        if let Some(node) = graph.first_unsupported_node() {
            return Err(WorkflowValidationError::UnsupportedNodeType {
                node_id: node.id.clone(),
                node_type: node.node_type,
            }
            .into());
        }
        let unreachable = graph.unreachable_from_start();
        if !unreachable.is_empty() {
            return Err(WorkflowValidationError::UnreachableNodes {
                node_ids: unreachable,
            }
            .into());
        }
        validate_executable_graph(&graph)?;
        validate_start_inputs(start_node, context.run.payload.as_deref())?;
        let start_node_run = NodeRunToStart {
            id: self.node_run_id_generator.generate_node_run_id(),
            node_id: start_node.id.clone(),
            node_type: start_node.node_type.as_str().to_string(),
            input: self.runtimes.start_input(start_node, &context),
            iteration: None,
        };
        let now = self.clock.now_timestamp_millis();
        match self.repository.start_run(run_id, &start_node_run, now)? {
            StartWorkflowRunResult::Started => {
                self.run_events.publish_run_invalidated(run_id);
                self.run_schedule(run_id)?;
                Ok(StartWorkflowRunResult::Started)
            }
            StartWorkflowRunResult::Current => Ok(StartWorkflowRunResult::Current),
            StartWorkflowRunResult::NotFound => Err(EngineError::WorkflowRunNotFound {
                run_id: run_id.to_string(),
            }),
        }
    }

    /// Cancels a running run. The backend orchestrates stopping the run's live sessions around
    /// this; the `Cancelled` transition is committed here, and a late session stop makes the
    /// executor's in-flight callbacks no-ops against the already-cancelled node runs.
    pub fn cancel(&self, run_id: &WorkflowRunId) -> Result<CancelWorkflowRunResult, EngineError> {
        let now = self.clock.now_timestamp_millis();
        let result = self.repository.cancel_run(run_id, now)?;
        if matches!(result, CancelWorkflowRunResult::Cancelled) {
            self.run_events.publish_run_invalidated(run_id);
        }
        Ok(result)
    }

    /// Restarts a non-running run by resetting it and re-running it immediately.
    pub fn restart(&self, run_id: &WorkflowRunId) -> Result<RestartWorkflowRunResult, EngineError> {
        let now = self.clock.now_timestamp_millis();
        match self.repository.restart_run(run_id, now)? {
            RestartWorkflowRunResult::Restarted => {
                self.run_events.publish_run_invalidated(run_id);
                self.start(run_id)?;
                Ok(RestartWorkflowRunResult::Restarted)
            }
            result @ (RestartWorkflowRunResult::NotRestartable
            | RestartWorkflowRunResult::NotFound) => Ok(result),
        }
    }

    /// Sets the kickoff input of a `Pending` run so its start node receives it on start.
    pub fn update_run_input(
        &self,
        run_id: &WorkflowRunId,
        input: Option<String>,
        variables: std::collections::BTreeMap<String, serde_json::Value>,
    ) -> Result<UpdateWorkflowRunInputResult, EngineError> {
        let now = self.clock.now_timestamp_millis();
        Ok(self
            .repository
            .update_run_input(run_id, input, variables, now)?)
    }

    /// Marks one node-run succeeded and continues the scheduling wave.
    ///
    /// Late or duplicate callbacks are rejected idempotently by the repository and become no-ops.
    pub fn complete_node(
        &self,
        run_id: &WorkflowRunId,
        node_run_id: &WorkflowNodeRunId,
        output: Option<String>,
        structured_output: Option<serde_json::Value>,
        stop_reason: Option<String>,
        file_changes: Vec<FileChange>,
    ) -> Result<(), EngineError> {
        let now = self.clock.now_timestamp_millis();
        match self.repository.complete_node(
            node_run_id,
            output,
            structured_output,
            stop_reason,
            file_changes,
            now,
        )? {
            AdvanceWorkflowRunResult::Advanced => {
                self.run_events.publish_run_invalidated(run_id);
                self.run_schedule(run_id)
            }
            AdvanceWorkflowRunResult::NotRunning | AdvanceWorkflowRunResult::NotFound => Ok(()),
        }
    }

    /// Marks one node-run and its run failed; the run is terminal so no scheduling follows.
    ///
    /// A failure inside a `continue`-strategy iteration region is absorbed instead: the row
    /// fails, the run stays active, and the composite runtime settles the failed round on its
    /// next advance (ADR "iteration composite runtime" D6).
    pub fn fail_node(
        &self,
        run_id: &WorkflowRunId,
        node_run_id: &WorkflowNodeRunId,
        error: String,
        output: Option<String>,
    ) -> Result<(), EngineError> {
        let now = self.clock.now_timestamp_millis();
        let propagation = self.failure_propagation(run_id, node_run_id)?;
        match self
            .repository
            .fail_node(node_run_id, error, output, propagation, now)?
        {
            AdvanceWorkflowRunResult::Advanced => {
                self.run_events.publish_run_invalidated(run_id);
                if propagation == FailurePropagation::Composite {
                    // The absorbing runtime must settle the failed round and advance.
                    self.run_schedule(run_id)?;
                }
            }
            AdvanceWorkflowRunResult::NotRunning | AdvanceWorkflowRunResult::NotFound => {}
        }
        Ok(())
    }

    /// Resumes scheduling for a `Running` run left with no active node by a crash between a node
    /// completion and its successor scheduling. Recomputes the ready set from persisted state:
    /// either a ready successor is dispatched or the drained run is finished.
    pub fn resume(&self, run_id: &WorkflowRunId) -> Result<(), EngineError> {
        self.run_schedule(run_id)
    }

    /// Runs one reactive scheduling pass: complete in-flight swift nodes, advance composite
    /// nodes, dispatch ready nodes, and finish the run once the graph is drained.
    ///
    /// This is the scheduling core: it resolves every node through the runtime registry and
    /// never matches on node types itself. In-flight nodes without a background driver — the
    /// swift runtimes — complete synchronously against the committed pool; async runtimes are
    /// owned by their background drivers and report through the callback sink instead;
    /// composite runtimes are handed their Running rows each wave and answered with a pure
    /// plan the engine executes (ADR "node runtime orchestration" D5 advance coordination).
    fn run_schedule(&self, run_id: &WorkflowRunId) -> Result<(), EngineError> {
        let now = self.clock.now_timestamp_millis();
        loop {
            let context = self.execution_context(run_id)?;
            let graph = WorkflowGraph::parse(&context.graph_json)?;
            let node_runs = self.repository.list_node_runs(run_id)?;
            let (pool, _) = execution_state_from(context.run.payload.as_deref());

            let mut completed_swift = false;
            for node_run in node_runs
                .iter()
                .filter(|node_run| node_run.status == WorkflowNodeStatus::Running)
            {
                let Some(node) = graph.node(&node_run.node_id) else {
                    continue;
                };
                let Some(RegisteredNodeRuntime::Swift(runtime)) =
                    self.runtimes.runtime(node.node_type)
                else {
                    continue;
                };
                let completion = SwiftCompletion {
                    node_run,
                    run_input: context.run.input.as_deref(),
                    pool: &pool,
                    node_runs: &node_runs,
                };
                match runtime.complete_running(node, &completion) {
                    Ok(output) => {
                        let advanced = self.repository.complete_node(
                            &node_run.id,
                            Some(output),
                            None,
                            None,
                            Vec::new(),
                            now,
                        )?;
                        if matches!(advanced, AdvanceWorkflowRunResult::Advanced) {
                            self.run_events.publish_run_invalidated(run_id);
                        }
                        completed_swift = true;
                    }
                    Err(message) => {
                        let propagation = region_failure_propagation(&graph, &node_run.node_id);
                        let advanced = self.repository.fail_node(
                            &node_run.id,
                            message,
                            None,
                            propagation,
                            now,
                        )?;
                        if matches!(advanced, AdvanceWorkflowRunResult::Advanced) {
                            self.run_events.publish_run_invalidated(run_id);
                        }
                        if propagation == FailurePropagation::Composite {
                            // The absorbed failure settles as a failed round on the next pass.
                            continue;
                        }
                        return Ok(());
                    }
                }
            }

            // Swift completions persist internal routing state; reload before projecting branches.
            if completed_swift {
                continue;
            }

            // Advance coordination: hand every Running composite row back to its runtime. A
            // non-noop plan commits a transition, so the loop reloads and re-plans until the
            // plan settles into waiting on in-flight region rows or the outer ready set.
            let advanced_composite =
                self.advance_composites(run_id, &graph, &node_runs, &context, now)?;
            if advanced_composite {
                continue;
            }

            let context = self.execution_context(run_id)?;
            let (_, condition_decisions) = execution_state_from(context.run.payload.as_deref());
            let node_runs = self.repository.list_node_runs(run_id)?;
            let projection = BranchProjection::new(&graph, &node_runs, &condition_decisions);
            let ready: Vec<&WorkflowGraphNode> = projection.ready_nodes();

            if ready.is_empty() {
                if !projection.has_in_flight() {
                    let output = self.runtimes.compute_run_output(&node_runs);
                    self.repository.finish_run(run_id, output, now)?;
                    self.run_events.publish_run_invalidated(run_id);
                }
                return Ok(());
            }

            let ready_runs: Vec<NodeRunToStart> = ready
                .iter()
                .map(|node| NodeRunToStart {
                    id: self.node_run_id_generator.generate_node_run_id(),
                    node_id: node.id.clone(),
                    node_type: node.node_type.as_str().to_string(),
                    input: self.runtimes.start_input(node, &context),
                    iteration: None,
                })
                .collect();
            self.repository
                .start_ready_nodes(run_id, &ready_runs, now)?;
            self.run_events.publish_run_invalidated(run_id);

            // Async runtimes dispatch now; swift runtimes complete on the next loop iteration.
            for (node, node_run) in ready.iter().zip(ready_runs.iter()) {
                if let Some(RegisteredNodeRuntime::Async(runtime)) =
                    self.runtimes.runtime(node.node_type)
                {
                    runtime.dispatch(&node_run.id, node, &context);
                }
            }
        }
    }

    /// Plans and executes one advance step for every Running composite node-run of the run.
    ///
    /// Returns whether any transition committed (so the caller reloads and re-plans). The plan
    /// is pure; execution goes through the repository's atomic composite operations, and any
    /// rows the plan started are dispatched to their async runtimes before returning.
    fn advance_composites(
        &self,
        run_id: &WorkflowRunId,
        graph: &WorkflowGraph,
        node_runs: &[WorkflowNodeRun],
        context: &ExecutionContext,
        now: i64,
    ) -> Result<bool, EngineError> {
        let payload = execution_payload_from(context.run.payload.as_deref());
        for node_run in node_runs
            .iter()
            .filter(|node_run| node_run.status == WorkflowNodeStatus::Running)
        {
            let Some(node) = graph.node(&node_run.node_id) else {
                continue;
            };
            let Some(RegisteredNodeRuntime::Composite(runtime)) =
                self.runtimes.runtime(node.node_type)
            else {
                continue;
            };
            let plan = match runtime.plan_advance(node, graph, node_runs, &payload) {
                Ok(plan) => plan,
                // A composite-own failure always propagates to the run (ADR D8).
                Err(error) => {
                    self.repository.fail_node(
                        &node_run.id,
                        error,
                        None,
                        FailurePropagation::Run,
                        now,
                    )?;
                    self.run_events.publish_run_invalidated(run_id);
                    return Ok(true);
                }
            };
            let started = self
                .execute_composite_plan(run_id, graph, context, node_run, node_runs, plan, now)?;
            if started.is_some() {
                self.run_events.publish_run_invalidated(run_id);
                // Dispatch the rows this plan started to their background drivers.
                if let Some((_, started_runs)) = started {
                    for planned in &started_runs {
                        if let Some(started_node) = graph.node(&planned.node_id)
                            && let Some(RegisteredNodeRuntime::Async(runtime)) =
                                self.runtimes.runtime(started_node.node_type)
                        {
                            runtime.dispatch(&planned.id, started_node, context);
                        }
                    }
                }
                // One transition per pass: reload rows and re-plan with fresh facts.
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Executes one composite advance plan through the repository's atomic operations.
    ///
    /// Returns the round and materialized rows the plan started, so the caller can dispatch
    /// their async runtimes; `None` means the plan committed no transition (a no-op).
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    fn execute_composite_plan(
        &self,
        run_id: &WorkflowRunId,
        graph: &WorkflowGraph,
        context: &ExecutionContext,
        node_run: &WorkflowNodeRun,
        node_runs: &[WorkflowNodeRun],
        plan: CompositeAdvancePlan,
        now: i64,
    ) -> Result<Option<(Option<u32>, Vec<NodeRunToStart>)>, EngineError> {
        let materialize = |round: Option<u32>, node_ids: &[String]| -> Vec<NodeRunToStart> {
            node_ids
                .iter()
                .filter_map(|node_id| {
                    let node = graph.node(node_id)?;
                    Some(NodeRunToStart {
                        id: self.node_run_id_generator.generate_node_run_id(),
                        node_id: node.id.clone(),
                        node_type: node.node_type.as_str().to_string(),
                        input: self.runtimes.start_input(node, context),
                        iteration: round,
                    })
                })
                .collect()
        };
        match plan {
            CompositeAdvancePlan::Noop => Ok(None),
            CompositeAdvancePlan::StartRound {
                round,
                item,
                node_ids,
            } => {
                let rows = materialize(Some(round), &node_ids);
                let result = self.repository.start_iteration_round(
                    run_id,
                    &node_run.node_id,
                    round,
                    &item,
                    &rows,
                    now,
                )?;
                Ok(matches!(result, AdvanceWorkflowRunResult::Advanced)
                    .then_some((Some(round), rows)))
            }
            CompositeAdvancePlan::StartRegionNodes { node_ids } => {
                // Mid-round wave: the rows carry the round they belong to, derived from the
                // region's persisted rows so no in-memory round state exists (ADR D2).
                let round = region_rows_round(graph, node_run, node_runs);
                let rows = materialize(round, &node_ids);
                self.repository.start_ready_nodes(run_id, &rows, now)?;
                Ok(Some((round, rows)))
            }
            CompositeAdvancePlan::SettleRound {
                round,
                entry,
                continuation,
            } => {
                let continuation = match continuation {
                    CompositeContinuation::StartNextRound {
                        round,
                        item,
                        node_ids,
                    } => IterationRoundContinuation::StartNextRound {
                        round,
                        item,
                        node_runs: materialize(Some(round), &node_ids),
                    },
                    CompositeContinuation::Complete { exposed, output } => {
                        IterationRoundContinuation::Complete { exposed, output }
                    }
                    CompositeContinuation::Fail { error } => {
                        IterationRoundContinuation::Fail { error }
                    }
                };
                let result = self.repository.settle_iteration_round(
                    run_id,
                    &node_run.node_id,
                    round,
                    entry,
                    continuation,
                    now,
                )?;
                Ok(matches!(result, AdvanceWorkflowRunResult::Advanced)
                    .then_some((None, Vec::new())))
            }
            CompositeAdvancePlan::CompleteNode { exposed, output } => {
                let result = self.repository.complete_iteration_node(
                    &node_run.id,
                    &node_run.node_id,
                    &exposed,
                    output,
                    now,
                )?;
                Ok(matches!(result, AdvanceWorkflowRunResult::Advanced)
                    .then_some((None, Vec::new())))
            }
            CompositeAdvancePlan::FailNode { error } => {
                let result = self.repository.fail_node(
                    &node_run.id,
                    error,
                    None,
                    FailurePropagation::Run,
                    now,
                )?;
                Ok(matches!(result, AdvanceWorkflowRunResult::Advanced)
                    .then_some((None, Vec::new())))
            }
        }
    }

    /// Resolves how one node-run's failure propagates, by graph structure only (ADR
    /// "iteration composite runtime" D6): a failure inside a `continue`-strategy composite
    /// region is absorbed by the runtime; everything else keeps the `Run` behavior.
    fn failure_propagation(
        &self,
        run_id: &WorkflowRunId,
        node_run_id: &WorkflowNodeRunId,
    ) -> Result<FailurePropagation, EngineError> {
        let node_run = self.repository.find_node_run_by_id(node_run_id)?.ok_or(
            EngineError::WorkflowRunNotFound {
                run_id: run_id.to_string(),
            },
        )?;
        if node_run.iteration.is_none() {
            return Ok(FailurePropagation::Run);
        }
        let context = self.execution_context(run_id)?;
        let graph = WorkflowGraph::parse(&context.graph_json)?;
        Ok(region_failure_propagation(&graph, &node_run.node_id))
    }

    /// Loads the execution context or reports the run as missing.
    fn execution_context(&self, run_id: &WorkflowRunId) -> Result<ExecutionContext, EngineError> {
        self.repository
            .find_execution_context(run_id)?
            .ok_or_else(|| EngineError::WorkflowRunNotFound {
                run_id: run_id.to_string(),
            })
    }
}

/// Enforces form-level Start constraints at the execution boundary, not only in the editor.
fn validate_start_inputs(
    start_node: &WorkflowGraphNode,
    serialized_payload: Option<&str>,
) -> Result<(), WorkflowValidationError> {
    let variable_pool = serialized_payload
        .and_then(|payload| serde_json::from_str::<WorkflowRunPayload>(payload).ok())
        .map(|payload| payload.variable_pool)
        .unwrap_or_default();
    for variable in &start_node.input_variables {
        let selector = format!("{}.{}", start_node.id, variable.name);
        let value = variable_pool
            .values
            .get(&selector)
            .or(variable.value.as_ref());
        let missing = value.is_none_or(|value| {
            value.is_null()
                || value.as_str().is_some_and(str::is_empty)
                || value.as_array().is_some_and(Vec::is_empty)
        });
        if variable.required && missing {
            return Err(WorkflowValidationError::MissingRequiredStartVariable {
                name: variable.name.clone(),
            });
        }
        if !variable.options.is_empty()
            && value
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| !variable.options.iter().any(|option| option == value))
        {
            return Err(WorkflowValidationError::InvalidStartVariableOption {
                name: variable.name.clone(),
            });
        }
    }
    Ok(())
}

/// Validates the structural invariants that make a graph executable: a terminal output, no edges
/// leaving an output, unique condition case ids, real condition branch handles, and result names
/// that are unique within each output node.
///
/// This is start-time graph-structural policy, not scheduling: it inspects node kinds the same
/// way `WorkflowGraph::parse` does, so it stays here rather than behind the runtime registry.
fn validate_executable_graph(graph: &WorkflowGraph) -> Result<(), WorkflowValidationError> {
    for node in graph.nodes() {
        match node.node_type {
            NodeType::Output => {
                if !graph.successors(&node.id).is_empty() {
                    return Err(WorkflowValidationError::OutputNodeHasSuccessors {
                        node_id: node.id.clone(),
                    });
                }
                if let Some(config) = &node.output_config {
                    let mut output_names = HashSet::new();
                    for binding in &config.outputs {
                        if !output_names.insert(&binding.name) {
                            return Err(WorkflowValidationError::DuplicateOutputName {
                                node_id: node.id.clone(),
                                name: binding.name.clone(),
                            });
                        }
                    }
                }
            }
            NodeType::Condition => {
                if let Some(config) = &node.condition_config {
                    let mut seen_cases = HashSet::new();
                    for case in &config.cases {
                        if !seen_cases.insert(&case.id) {
                            return Err(WorkflowValidationError::DuplicateConditionCase {
                                node_id: node.id.clone(),
                                case_id: case.id.clone(),
                            });
                        }
                    }
                    for edge in graph.outgoing_edges(&node.id) {
                        let handle = edge.source_handle.as_deref().unwrap_or(ELSE_BRANCH_ID);
                        let valid = handle == ELSE_BRANCH_ID
                            || config.cases.iter().any(|case| case.id == handle);
                        if !valid {
                            return Err(WorkflowValidationError::UnknownConditionBranch {
                                node_id: node.id.clone(),
                                handle: handle.to_string(),
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Loads user variables and private routing decisions, including decisions from legacy pools.
fn execution_state_from(
    serialized_payload: Option<&str>,
) -> (
    WorkflowVariablePool,
    std::collections::BTreeMap<String, String>,
) {
    let Some(payload) = serialized_payload
        .and_then(|payload| serde_json::from_str::<WorkflowRunPayload>(payload).ok())
    else {
        return (WorkflowVariablePool::default(), Default::default());
    };
    let condition_decisions = payload.resolved_condition_decisions();
    (payload.variable_pool, condition_decisions)
}

/// Loads the full run payload, defaulting when a run carries none.
fn execution_payload_from(serialized_payload: Option<&str>) -> WorkflowRunPayload {
    serialized_payload
        .and_then(|payload| serde_json::from_str::<WorkflowRunPayload>(payload).ok())
        .unwrap_or_default()
}

/// Resolves how a failure of the node with the given id propagates, structurally: any failure
/// inside a composite region resolves to the owning composite node with `Composite` semantics
/// (the row fails, the run stays), and the owner's error strategy decides the node's fate at
/// settlement — `fail` fails the node and the run there, `continue` records the round and
/// advances (ADR "iteration composite runtime" D4, D6). This is a graph-structure judgment,
/// not a node-type branch in the scheduling core.
fn region_failure_propagation(graph: &WorkflowGraph, node_id: &str) -> FailurePropagation {
    match graph.region_owner(node_id) {
        Some(_) => FailurePropagation::Composite,
        None => FailurePropagation::Run,
    }
}

/// Derives the round a composite node's region is currently executing, from the region's
/// persisted rows only (v1 serial execution; ADR "iteration composite runtime" D2).
fn region_rows_round(
    graph: &WorkflowGraph,
    node_run: &WorkflowNodeRun,
    node_runs: &[WorkflowNodeRun],
) -> Option<u32> {
    let region = graph.region(&node_run.node_id)?;
    node_runs
        .iter()
        .filter(|row| row.iteration.is_some() && region.contains(&row.node_id))
        .filter_map(|row| row.iteration)
        .max()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_run::engine::variable_pool::WorkflowVariablePool;
    use ora_contracts::WorkflowRunLocale;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    // ── Executable-graph validation ──

    fn graph(json: serde_json::Value) -> WorkflowGraph {
        WorkflowGraph::parse(&json.to_string()).unwrap()
    }

    #[test]
    fn validation_rejects_an_output_with_successors() {
        let g = graph(json!({
            "nodes": [
                { "id": "out", "data": { "kind": "output" } },
                { "id": "a", "data": { "kind": "agent", "agentConfig": {
                    "executor": { "agentCli": "c", "modelId": "m" }, "roleId": "R", "skills": [], "prompt": "a"
                } } }
            ],
            "edges": [{ "source": "out", "target": "a" }]
        }));
        assert_eq!(
            validate_executable_graph(&g).unwrap_err(),
            WorkflowValidationError::OutputNodeHasSuccessors {
                node_id: "out".to_string()
            }
        );
    }

    #[test]
    fn validation_rejects_duplicate_condition_case_ids() {
        let g = graph(json!({
            "nodes": [
                { "id": "start", "data": { "kind": "start" } },
                { "id": "out", "data": { "kind": "output" } },
                { "id": "c", "data": { "kind": "condition", "cases": [
                    { "id": "x", "logic": "and", "conditions": [] },
                    { "id": "x", "logic": "and", "conditions": [] }
                ] } }
            ],
            "edges": []
        }));
        assert_eq!(
            validate_executable_graph(&g).unwrap_err(),
            WorkflowValidationError::DuplicateConditionCase {
                node_id: "c".to_string(),
                case_id: "x".to_string()
            }
        );
    }

    #[test]
    fn validation_rejects_an_unknown_condition_branch_handle() {
        let g = graph(json!({
            "nodes": [
                { "id": "start", "data": { "kind": "start" } },
                { "id": "c", "data": { "kind": "condition", "cases": [
                    { "id": "approved", "logic": "and", "conditions": [] }
                ] } },
                { "id": "out", "data": { "kind": "output" } }
            ],
            "edges": [
                { "source": "start", "target": "c" },
                { "source": "c", "sourceHandle": "bogus", "target": "out" }
            ]
        }));
        assert_eq!(
            validate_executable_graph(&g).unwrap_err(),
            WorkflowValidationError::UnknownConditionBranch {
                node_id: "c".to_string(),
                handle: "bogus".to_string()
            }
        );
    }

    /// Separate terminal paths may expose the same public result shape.
    #[test]
    fn validation_allows_the_same_result_name_on_separate_output_nodes() {
        let g = graph(json!({
            "nodes": [
                { "id": "start", "data": { "kind": "start" } },
                { "id": "a", "data": { "kind": "output", "outputs": [
                    { "name": "result", "variableSelector": ["x", "text"] }
                ] } },
                { "id": "b", "data": { "kind": "output", "outputs": [
                    { "name": "result", "variableSelector": ["y", "text"] }
                ] } }
            ],
            "edges": []
        }));
        assert!(validate_executable_graph(&g).is_ok());
    }

    /// Duplicate names in one Output would overwrite each other in its JSON object.
    #[test]
    fn validation_rejects_duplicate_result_names_within_one_output_node() {
        let g = graph(json!({
            "nodes": [
                { "id": "start", "data": { "kind": "start" } },
                { "id": "out", "data": { "kind": "output", "outputs": [
                    { "name": "result", "variableSelector": ["x", "text"] },
                    { "name": "result", "variableSelector": ["y", "text"] }
                ] } }
            ],
            "edges": []
        }));
        assert_eq!(
            validate_executable_graph(&g).unwrap_err(),
            WorkflowValidationError::DuplicateOutputName {
                node_id: "out".to_string(),
                name: "result".to_string()
            }
        );
    }

    #[test]
    fn validation_accepts_a_branching_terminated_graph() {
        let g = graph(json!({
            "nodes": [
                { "id": "start", "data": { "kind": "start" } },
                { "id": "c", "data": { "kind": "condition", "cases": [
                    { "id": "approved", "logic": "and", "conditions": [] }
                ] } },
                { "id": "ok", "data": { "kind": "output" } },
                { "id": "no", "data": { "kind": "output" } }
            ],
            "edges": [
                { "source": "start", "target": "c" },
                { "source": "c", "sourceHandle": "approved", "target": "ok" },
                { "source": "c", "sourceHandle": "else", "target": "no" }
            ]
        }));
        assert!(validate_executable_graph(&g).is_ok());
    }

    /// Required Start fields are enforced again when execution begins, even if IPC is bypassed.
    #[test]
    fn validation_requires_start_values_at_execution() {
        let graph = graph(json!({
            "nodes": [{
                "id": "start",
                "data": {
                    "kind": "start",
                    "inputVariables": [{
                        "name": "brief",
                        "fieldType": "paragraph",
                        "valueType": "string",
                        "required": true
                    }]
                }
            }],
            "edges": []
        }));
        let start = graph.start_node().unwrap();
        assert_eq!(
            validate_start_inputs(start, None).unwrap_err(),
            WorkflowValidationError::MissingRequiredStartVariable {
                name: "brief".to_string()
            }
        );

        let mut variable_pool = WorkflowVariablePool::from_graph(&graph);
        variable_pool
            .set("start.brief", "start", json!("Ship it"))
            .unwrap();
        let payload = WorkflowRunPayload::with_variable_pool(
            WorkflowRunLocale::EnUs,
            Default::default(),
            Some("start".to_string()),
            variable_pool,
        );
        assert!(
            validate_start_inputs(start, Some(&serde_json::to_string(&payload).unwrap())).is_ok()
        );
    }
}
