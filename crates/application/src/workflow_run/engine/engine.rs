use crate::RepositoryError;
use crate::project::Clock;
use crate::workflow_run::engine::branch_projection::BranchProjection;
use crate::workflow_run::engine::condition::{ConditionError, ELSE_BRANCH_ID, evaluate_condition};
use crate::workflow_run::engine::graph::{GraphError, WorkflowGraph, WorkflowGraphNode};
use crate::workflow_run::engine::node_type::NodeType;
use crate::workflow_run::engine::ports::{
    AdvanceWorkflowRunResult, CancelWorkflowRunResult, ExecutionContext, FileChange,
    NodeRunToStart, RestartWorkflowRunResult, StartWorkflowRunResult, UpdateWorkflowRunInputResult,
    WorkflowNodeRunIdGenerator, WorkflowRunEngineRepository,
};
use crate::workflow_run::engine::skill_delivery::WorkflowRunPayload;
use crate::workflow_run::engine::variable_pool::WorkflowVariablePool;
use ora_domain::{WorkflowNodeRun, WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRunId};
use std::collections::HashSet;
use thiserror::Error;

/// Executes one agent node through a real session, calling the engine back when done.
///
/// The implementation lives in the backend and drives the session asynchronously; it MUST report
/// completion through `WorkflowRunEngine::complete_node`/`fail_node` on the same per-run serial
/// executor so state transitions stay serial.
pub trait NodeExecutor {
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
/// and ready sets from persistence. Agent execution is delegated through `NodeExecutor`; the
/// backend must route all commands and callbacks for one run through a single serial executor.
#[derive(Clone)]
pub struct WorkflowRunEngine<R, E, G, C> {
    repository: R,
    node_executor: E,
    node_run_id_generator: G,
    clock: C,
}

impl<R, E, G, C> WorkflowRunEngine<R, E, G, C> {
    /// Builds an engine from its ports.
    pub fn new(repository: R, node_executor: E, node_run_id_generator: G, clock: C) -> Self {
        Self {
            repository,
            node_executor,
            node_run_id_generator,
            clock,
        }
    }
}

impl<R, E, G, C> WorkflowRunEngine<R, E, G, C>
where
    R: WorkflowRunEngineRepository,
    E: NodeExecutor,
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
            input: context.run.input,
        };
        let now = self.clock.now_timestamp_millis();
        match self.repository.start_run(run_id, &start_node_run, now)? {
            StartWorkflowRunResult::Started => {
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
        Ok(self.repository.cancel_run(run_id, now)?)
    }

    /// Restarts a non-running run by resetting it and re-running it immediately.
    pub fn restart(&self, run_id: &WorkflowRunId) -> Result<RestartWorkflowRunResult, EngineError> {
        let now = self.clock.now_timestamp_millis();
        match self.repository.restart_run(run_id, now)? {
            RestartWorkflowRunResult::Restarted => {
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
            AdvanceWorkflowRunResult::Advanced => self.run_schedule(run_id),
            AdvanceWorkflowRunResult::NotRunning | AdvanceWorkflowRunResult::NotFound => Ok(()),
        }
    }

    /// Marks one node-run and its run failed; the run is terminal so no scheduling follows.
    pub fn fail_node(
        &self,
        node_run_id: &WorkflowNodeRunId,
        error: String,
        output: Option<String>,
    ) -> Result<(), EngineError> {
        let now = self.clock.now_timestamp_millis();
        match self.repository.fail_node(node_run_id, error, output, now)? {
            AdvanceWorkflowRunResult::Advanced
            | AdvanceWorkflowRunResult::NotRunning
            | AdvanceWorkflowRunResult::NotFound => Ok(()),
        }
    }

    /// Resumes scheduling for a `Running` run left with no active node by a crash between a node
    /// completion and its successor scheduling. Recomputes the ready set from persisted state:
    /// either a ready successor is dispatched or the drained run is finished.
    pub fn resume(&self, run_id: &WorkflowRunId) -> Result<(), EngineError> {
        self.run_schedule(run_id)
    }

    /// Runs one reactive scheduling pass: complete in-flight control nodes, dispatch ready nodes,
    /// and finish the run once the graph is drained.
    fn run_schedule(&self, run_id: &WorkflowRunId) -> Result<(), EngineError> {
        let now = self.clock.now_timestamp_millis();
        loop {
            let context = self.execution_context(run_id)?;
            let graph = WorkflowGraph::parse(&context.graph_json)?;
            let node_runs = self.repository.list_node_runs(run_id)?;
            let (pool, _) = execution_state_from(context.run.payload.as_deref());

            // In-flight control nodes have no session to call back; complete them synchronously.
            // A condition evaluates against the committed pool and reports its selected branch.
            let mut completed_control = false;
            for node_run in node_runs
                .iter()
                .filter(|node_run| node_run.status == WorkflowNodeStatus::Running)
            {
                let Some(node) = graph.node(&node_run.node_id) else {
                    continue;
                };
                match node.node_type {
                    NodeType::Start => {
                        let output = context.run.input.clone().unwrap_or_default();
                        self.repository.complete_node(
                            &node_run.id,
                            Some(output),
                            None,
                            None,
                            Vec::new(),
                            now,
                        )?;
                        completed_control = true;
                    }
                    NodeType::Output => {
                        // A workflow must reach exactly one Output; a second one completing means
                        // the active path degenerated into two terminals.
                        let current_run_id = node_run.id.clone();
                        let duplicate = node_runs.iter().find(|candidate| {
                            candidate.id != current_run_id
                                && candidate.node_type == "output"
                                && candidate.status == WorkflowNodeStatus::Succeeded
                        });
                        if let Some(previous) = duplicate {
                            let message = format!(
                                "multiple active output nodes: {} and {}",
                                previous.node_id, node.id
                            );
                            self.repository
                                .fail_node(&node_run.id, message, None, now)?;
                            return Ok(());
                        }
                        match control_node_output(node, &context, &pool) {
                            Ok(output) => {
                                self.repository.complete_node(
                                    &node_run.id,
                                    Some(output),
                                    None,
                                    None,
                                    Vec::new(),
                                    now,
                                )?;
                                completed_control = true;
                            }
                            Err(message) => {
                                self.repository
                                    .fail_node(&node_run.id, message, None, now)?;
                                return Ok(());
                            }
                        }
                    }
                    NodeType::Condition => {
                        let outcome = node
                            .condition_config
                            .as_ref()
                            .map(|config| evaluate_condition(config, &pool))
                            .unwrap_or_else(|| Err(ConditionError::MissingConfig));
                        match outcome {
                            Ok(selected) => {
                                self.repository.complete_node(
                                    &node_run.id,
                                    Some(selected),
                                    None,
                                    None,
                                    Vec::new(),
                                    now,
                                )?;
                                completed_control = true;
                            }
                            Err(error) => {
                                // A condition that reads an unset or invalid variable fails the
                                // node and the run rather than guessing a branch.
                                self.repository.fail_node(
                                    &node_run.id,
                                    error.to_string(),
                                    None,
                                    now,
                                )?;
                                return Ok(());
                            }
                        }
                    }
                    _ => {}
                }
            }

            // Control completions persist internal routing state; reload before projecting branches.
            if completed_control {
                continue;
            }

            let context = self.execution_context(run_id)?;
            let (_, condition_decisions) = execution_state_from(context.run.payload.as_deref());
            let node_runs = self.repository.list_node_runs(run_id)?;
            let projection = BranchProjection::new(&graph, &node_runs, &condition_decisions);
            let ready: Vec<&WorkflowGraphNode> = projection.ready_nodes();

            if ready.is_empty() {
                if !projection.has_in_flight() {
                    let output = compute_run_output(&node_runs);
                    self.repository.finish_run(run_id, output, now)?;
                }
                return Ok(());
            }

            let ready_runs: Vec<NodeRunToStart> = ready
                .iter()
                .map(|node| NodeRunToStart {
                    id: self.node_run_id_generator.generate_node_run_id(),
                    node_id: node.id.clone(),
                    node_type: node.node_type.as_str().to_string(),
                    input: node_input(node, &context),
                })
                .collect();
            self.repository
                .start_ready_nodes(run_id, &ready_runs, now)?;

            // Control nodes complete on the next loop iteration; agent nodes dispatch now.
            for (node, node_run) in ready.iter().zip(ready_runs.iter()) {
                if node.node_type == NodeType::Agent {
                    self.node_executor.dispatch(&node_run.id, node, &context);
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

/// Computes the output a control node writes when it completes synchronously.
///
/// An Output node resolves only its explicitly declared variable bindings into a JSON object.
fn control_node_output(
    node: &WorkflowGraphNode,
    context: &ExecutionContext,
    pool: &WorkflowVariablePool,
) -> Result<String, String> {
    match node.node_type {
        NodeType::Start => Ok(context.run.input.clone().unwrap_or_default()),
        NodeType::Output => {
            if let Some(config) = &node.output_config {
                let mut result = serde_json::Map::new();
                for binding in &config.outputs {
                    let value = pool
                        .resolve(&binding.variable_selector)
                        .map_err(|error| error.to_string())?
                        .ok_or_else(|| {
                            format!(
                                "result {} references unassigned variable {}",
                                binding.name,
                                binding.variable_selector.qualified()
                            )
                        })?;
                    result.insert(binding.name.clone(), value.clone());
                }
                Ok(serde_json::Value::Object(result).to_string())
            } else {
                Ok(String::new())
            }
        }
        _ => Ok(String::new()),
    }
}

/// Validates the structural invariants that make a graph executable: a terminal output, no edges
/// leaving an output, unique condition case ids, real condition branch handles, and result names
/// that are unique within each output node.
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

/// Computes the run output written at finish: the last output node's output, or the last
/// completed agent's output when the graph has no output node.
fn compute_run_output(node_runs: &[WorkflowNodeRun]) -> Option<String> {
    let succeeded: Vec<&WorkflowNodeRun> = node_runs
        .iter()
        .filter(|node_run| node_run.status == WorkflowNodeStatus::Succeeded)
        .collect();
    if let Some(output_node) = succeeded
        .iter()
        .filter(|node_run| node_run.node_type == "output")
        .max_by_key(|node_run| node_run.finished_at.unwrap_or(0))
    {
        return output_node.output.clone();
    }
    let last_agent = succeeded
        .iter()
        .filter(|node_run| node_run.node_type == "agent")
        .max_by_key(|node_run| node_run.finished_at.unwrap_or(0));
    last_agent.and_then(|node_run| node_run.output.clone())
}

/// Computes the scalar input recorded on a node run when it starts.
fn node_input(node: &WorkflowGraphNode, context: &ExecutionContext) -> Option<String> {
    match node.node_type {
        NodeType::Start => context.run.input.clone(),
        NodeType::Agent => node
            .agent_config
            .as_ref()
            .map(|config| config.prompt.clone()),
        NodeType::Output | NodeType::Prompt | NodeType::Condition | NodeType::Tool => None,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_run::engine::variable_pool::WorkflowVariablePool;
    use ora_contracts::WorkflowRunLocale;
    use ora_domain::{
        AuditFields, WorkflowRun, WorkflowRunId, WorkflowRunStatus, Workspace, WorkspaceId,
        WorkspaceKind, WorkspaceLifecycle, WorkspaceLocation,
    };
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Builds a run context whose run input is fixed; workspace and graph are unused by the test.
    fn context() -> ExecutionContext {
        ExecutionContext {
            run: WorkflowRun::new(
                WorkflowRunId::new("run-1"),
                WorkspaceId::new("workspace-1"),
                ora_domain::WorkflowId::new("workflow-1"),
                ora_domain::WorkflowSnapshotId::new("snapshot-1"),
                "Review",
                WorkflowRunStatus::Running,
                None,
                Some("task".to_string()),
                None,
                None,
                None,
                None,
                None,
                AuditFields::new(1, 1, false),
            ),
            workspace: Workspace::new(
                WorkspaceId::new("workspace-1"),
                ora_domain::ProjectId::new("project-1"),
                WorkspaceKind::Main,
                WorkspaceLocation::local_filesystem("/tmp/workspace"),
                WorkspaceLifecycle::Active,
                AuditFields::new(1, 1, false),
            ),
            graph_json: String::new(),
        }
    }

    /// An output node with declared bindings resolves each named result from the variable pool.
    #[test]
    fn output_resolves_declared_bindings_from_the_pool() {
        let graph = WorkflowGraph::parse(
            r#"{
                "nodes": [
                    {"id":"review","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"review"}}},
                    {"id":"out","data":{"kind":"output","outputs":[
                        {"name":"approved","variableSelector":["review","structured_output","approved"]},
                        {"name":"files","variableSelector":["review","structured_output","files"]},
                        {"name":"summary","variableSelector":["review","text"]}
                    ]}}
                ],
                "edges": [{"source":"review","target":"out"}]
            }"#,
        )
        .unwrap();
        let mut pool = WorkflowVariablePool::default();
        pool.declare("review.structured_output", "object", "review");
        pool.declare("review.text", "string", "review");
        pool.set(
            "review.structured_output",
            "review",
            json!({
                "approved": true,
                "files": [{
                    "file_path": "src/vs/base/common/numbers.ts",
                    "lines": [{
                        "symbol": "formatTokenCount",
                        "start_line": 15,
                        "end_line": 26
                    }]
                }]
            }),
        )
        .unwrap();
        pool.set("review.text", "review", json!("ok")).unwrap();

        let node = graph.node("out").unwrap();
        let output = control_node_output(node, &context(), &pool).unwrap();
        assert_eq!(
            output,
            r#"{"approved":true,"files":[{"file_path":"src/vs/base/common/numbers.ts","lines":[{"symbol":"formatTokenCount","start_line":15,"end_line":26}]}],"summary":"ok"}"#
        );
    }

    /// An output binding that references an unassigned variable fails instead of emitting null.
    #[test]
    fn output_fails_when_a_binding_is_unassigned() {
        let graph = WorkflowGraph::parse(
            r#"{
                "nodes": [
                    {"id":"out","data":{"kind":"output","outputs":[
                        {"name":"summary","variableSelector":["writer","text"]}
                    ]}}
                ],
                "edges": []
            }"#,
        )
        .unwrap();
        let mut pool = WorkflowVariablePool::default();
        // The variable is declared by the graph but the writer has not produced it yet.
        pool.declare("writer.text", "string", "writer");

        let node = graph.node("out").unwrap();
        let error = control_node_output(node, &context(), &pool).unwrap_err();
        assert!(error.contains("unassigned variable writer.text"));
    }

    /// An output node without bindings does not import predecessor output implicitly.
    #[test]
    fn output_without_bindings_is_empty() {
        let graph = WorkflowGraph::parse(
            r#"{
                "nodes": [
                    {"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"a"}}},
                    {"id":"out","data":{"kind":"output"}}
                ],
                "edges": [{"source":"a","target":"out"}]
            }"#,
        )
        .unwrap();
        let node = graph.node("out").unwrap();
        let output =
            control_node_output(node, &context(), &WorkflowVariablePool::default()).unwrap();
        assert_eq!(output, "");
    }

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
