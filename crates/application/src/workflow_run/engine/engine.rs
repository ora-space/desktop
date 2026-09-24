use crate::project::Clock;
use crate::workflow_run::engine::branch_projection::BranchProjection;
use crate::workflow_run::engine::condition::ELSE_BRANCH_ID;
use crate::workflow_run::engine::failure::NodeFailure;
use crate::workflow_run::engine::graph::{WorkflowGraph, WorkflowGraphNode};
use crate::workflow_run::engine::node_executor::SharedNodeExecutor;
use crate::workflow_run::engine::node_runtime::{
    NodeRuntimeRegistry, RegisteredNodeRuntime, SwiftCompletion, standard_node_runtimes,
};
use crate::workflow_run::engine::node_type::NodeType;
use crate::workflow_run::engine::ports::{
    AdvanceWorkflowRunResult, CancelWorkflowRunResult, ExecutionContext, FailurePropagation,
    FileChange, NoRunInvalidations, NodeRunToStart, RestartWorkflowRunResult,
    ResumeWorkflowRunResult, StartWorkflowRunResult, UpdateWorkflowRunInputResult,
    WorkflowNodeRunIdGenerator, WorkflowRunEngineRepository, WorkflowRunInvalidationPublisher,
};
use crate::workflow_run::engine::region::region_failure_propagation;
use crate::workflow_run::engine::retry::{NoRetryTimer, WorkflowRetryTimer};
use crate::workflow_run::engine::skill_delivery::WorkflowRunPayload;
use crate::workflow_run::engine::start_input::validate_start_inputs;
use crate::workflow_run::engine::variable_pool::WorkflowVariablePool;
use ora_domain::{
    WorkflowNodeRun, WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRunId, WorkflowRunStatus,
};
use std::collections::HashSet;
use std::sync::Arc;

pub use super::node_executor::{
    EngineError, NodeExecutor, WorkflowRunCallback, WorkflowValidationError,
};

mod composite_scheduler;
mod loop_scheduler;
mod retry_scheduler;

/// Result of one scheduling pass inside a running Loop container.
enum LoopScheduleOutcome {
    Progressed,
    Waiting,
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
    node_executor: Arc<dyn NodeExecutor>,
    node_run_id_generator: G,
    clock: C,
    run_events: Arc<dyn WorkflowRunInvalidationPublisher>,
    retry_timer: Arc<dyn WorkflowRetryTimer>,
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
        let node_executor: Arc<dyn NodeExecutor> = Arc::new(agent_executor);
        Self {
            repository,
            runtimes: standard_node_runtimes(SharedNodeExecutor(node_executor.clone())),
            node_executor,
            node_run_id_generator,
            clock,
            run_events,
            retry_timer: Arc::new(NoRetryTimer),
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
            scope_id: context.root_scope_id.clone(),
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

    /// Resumes a `Failed`/`Cancelled` run from its failed nodes: every `Failed`/`Cancelled` node
    /// run and all of its transitive successors are cleared, succeeded work is kept, and
    /// scheduling recomputes the ready set from the surviving state.
    ///
    /// The resume unit for anything inside a region is the owning composite node. Partial
    /// in-loop resume is explicitly out of scope: a failed or cancelled region row, or a
    /// failed/cancelled composite row, restarts the loop from round 1.
    ///
    /// The invalidation is published after the resume transaction commits and before the
    /// scheduling wave, matching every other committed transition (ADR "node runtime
    /// orchestration" D7).
    pub fn resume_from_failure(
        &self,
        run_id: &WorkflowRunId,
    ) -> Result<ResumeWorkflowRunResult, EngineError> {
        let context = self.execution_context(run_id)?;
        if !matches!(
            context.run.status,
            WorkflowRunStatus::Failed | WorkflowRunStatus::Cancelled
        ) {
            return Ok(ResumeWorkflowRunResult::NotResumable);
        }
        let graph = WorkflowGraph::parse(&context.graph_json)?;
        let node_runs = self.repository.list_node_runs(run_id)?;
        let to_clear =
            crate::workflow_run::engine::region::resume_clear_node_ids(&graph, &node_runs);
        if to_clear.is_empty() {
            return Ok(ResumeWorkflowRunResult::NotResumable);
        }
        let now = self.clock.now_timestamp_millis();
        match self
            .repository
            .resume_from_failure(run_id, &to_clear, now)?
        {
            ResumeWorkflowRunResult::Resumed => {
                self.run_events.publish_run_invalidated(run_id);
                self.run_schedule(run_id)?;
                Ok(ResumeWorkflowRunResult::Resumed)
            }
            result
            @ (ResumeWorkflowRunResult::NotResumable | ResumeWorkflowRunResult::NotFound) => {
                Ok(result)
            }
        }
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
    /// next advance (ADR "iteration composite runtime" D6). A failure the node's automatic retry
    /// policy covers is replaced by a waiting attempt instead (see `retry_scheduler`). A callback
    /// for an attempt that a retry or resume already cleared is a no-op.
    pub fn fail_node(
        &self,
        run_id: &WorkflowRunId,
        node_run_id: &WorkflowNodeRunId,
        failure: NodeFailure,
    ) -> Result<(), EngineError> {
        let Some(node_run) = self.repository.find_node_run_by_id(node_run_id)? else {
            return Ok(());
        };
        let now = self.clock.now_timestamp_millis();
        if self.schedule_retry(run_id, &node_run, &failure, now) {
            return Ok(());
        }
        let propagation = self.failure_propagation(run_id, &node_run)?;
        match self
            .repository
            .fail_node(node_run_id, failure, propagation, now)?
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

    /// Records the git checkpoint taken before a node started, or why none could be taken.
    ///
    /// Provenance only: a missing node row is a no-op, matching the repository contract. This
    /// does not change run or node-run status, so it does not publish a run invalidation.
    pub fn record_node_checkpoint(
        &self,
        node_run_id: &WorkflowNodeRunId,
        snapshot_id: &str,
        checkpoint: Option<&str>,
        checkpoint_error: Option<&str>,
    ) -> Result<(), EngineError> {
        let now = self.clock.now_timestamp_millis();
        Ok(self.repository.record_node_checkpoint(
            node_run_id,
            snapshot_id,
            checkpoint,
            checkpoint_error,
            now,
        )?)
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
            // A Failed/Cancelled run must not dispatch dependents: a late sibling success
            // otherwise re-enters scheduling after fail_node has already promoted the run.
            if context.run.status != WorkflowRunStatus::Running {
                return Ok(());
            }
            let graph = WorkflowGraph::parse(&context.graph_json)?;
            let node_runs = self
                .repository
                .list_node_runs_in_scope(&context.root_scope_id)?;
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
                            NodeFailure::from_runtime(node.node_type, message),
                            propagation,
                            now,
                        )?;
                        if matches!(advanced, AdvanceWorkflowRunResult::Advanced) {
                            self.run_events.publish_run_invalidated(run_id);
                        }
                        if propagation == FailurePropagation::Composite {
                            // The absorbed failure settles as a failed round on the next pass.
                            completed_swift = true;
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
                    scope_id: context.root_scope_id.clone(),
                    node_id: node.id.clone(),
                    node_type: node.node_type.as_str().to_string(),
                    input: self.runtimes.start_input(node, &context),
                    iteration: None,
                })
                .collect();
            self.repository
                .start_ready_nodes(run_id, &ready_runs, now)?;
            self.run_events.publish_run_invalidated(run_id);

            // Async runtimes dispatch now; composite nodes record a pre-loop checkpoint;
            // swift runtimes complete on the next loop iteration.
            for (node, node_run) in ready.iter().zip(ready_runs.iter()) {
                match self.runtimes.runtime(node.node_type) {
                    Some(RegisteredNodeRuntime::Async(runtime)) => {
                        runtime.dispatch(
                            &node_run.id,
                            node,
                            &context,
                            &graph,
                            &context.root_scope_id,
                            &pool,
                        );
                    }
                    Some(RegisteredNodeRuntime::Composite(_)) => {
                        self.node_executor
                            .on_composite_node_started(&node_run.id, node, &context);
                    }
                    Some(RegisteredNodeRuntime::Swift(_) | RegisteredNodeRuntime::ScopedLoop)
                    | None => {}
                }
            }
        }
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
            NodeType::Loop => {
                if let Some((_, body)) = graph.loop_body(&node.id) {
                    validate_executable_graph(body)?;
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
            "edges": [{"source":"start","target":"c"}]
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
            "edges": [{"source":"start","target":"a"},{"source":"start","target":"b"}]
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
            "edges": [{"source":"start","target":"out"}]
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
