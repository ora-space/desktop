use crate::RepositoryError;
use crate::project::Clock;
use crate::workflow_run::engine::graph::{GraphError, WorkflowGraph, WorkflowGraphNode};
use crate::workflow_run::engine::node_type::NodeType;
use crate::workflow_run::engine::ports::{
    AdvanceWorkflowRunResult, CancelWorkflowRunResult, ExecutionContext, FileChange,
    NodeRunToStart, RestartWorkflowRunResult, StartWorkflowRunResult, UpdateWorkflowRunInputResult,
    WorkflowNodeRunIdGenerator, WorkflowRunEngineRepository,
};
use ora_domain::{WorkflowNodeRun, WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRunId};
use serde::Deserialize;
use std::collections::HashSet;
use thiserror::Error;

/// Separator between multiple direct-predecessor outputs concatenated by an output node.
pub const OUTPUT_PREDECESSOR_SEPARATOR: &str = "\n\n";

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
    /// Reports a successful node completion with its accumulated conversation, stop reason, and
    /// incremental file changes.
    fn complete_node(
        &self,
        run_id: &WorkflowRunId,
        node_run_id: &WorkflowNodeRunId,
        output: Option<String>,
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

/// One message in a node-run conversation array.
#[derive(Debug, Deserialize)]
struct ConversationEntry {
    role: String,
    text: String,
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
    ) -> Result<UpdateWorkflowRunInputResult, EngineError> {
        let now = self.clock.now_timestamp_millis();
        Ok(self.repository.update_run_input(run_id, input, now)?)
    }

    /// Marks one node-run succeeded and continues the scheduling wave.
    ///
    /// Late or duplicate callbacks are rejected idempotently by the repository and become no-ops.
    pub fn complete_node(
        &self,
        run_id: &WorkflowRunId,
        node_run_id: &WorkflowNodeRunId,
        output: Option<String>,
        stop_reason: Option<String>,
        file_changes: Vec<FileChange>,
    ) -> Result<(), EngineError> {
        let now = self.clock.now_timestamp_millis();
        match self
            .repository
            .complete_node(node_run_id, output, stop_reason, file_changes, now)?
        {
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

    /// Runs one reactive scheduling pass: complete in-flight control nodes, dispatch ready nodes,
    /// and finish the run once the graph is drained.
    fn run_schedule(&self, run_id: &WorkflowRunId) -> Result<(), EngineError> {
        let now = self.clock.now_timestamp_millis();
        loop {
            let context = self.execution_context(run_id)?;
            let graph = WorkflowGraph::parse(&context.graph_json)?;
            let mut node_runs = self.repository.list_node_runs(run_id)?;

            // In-flight control nodes have no session to call back; complete them synchronously.
            for node_run in node_runs
                .iter()
                .filter(|node_run| node_run.status == WorkflowNodeStatus::Running)
            {
                if let Some(node) = graph.node(&node_run.node_id)
                    && matches!(node.node_type, NodeType::Start | NodeType::Output)
                {
                    let output = control_node_output(&graph, node, &node_runs, &context);
                    self.repository.complete_node(
                        &node_run.id,
                        Some(output),
                        None,
                        Vec::new(),
                        now,
                    )?;
                }
            }

            node_runs = self.repository.list_node_runs(run_id)?;
            let completed: HashSet<&str> = node_runs
                .iter()
                .filter(|node_run| node_run.status == WorkflowNodeStatus::Succeeded)
                .map(|node_run| node_run.node_id.as_str())
                .collect();
            let in_flight: HashSet<&str> = node_runs
                .iter()
                .filter(|node_run| node_run.status == WorkflowNodeStatus::Running)
                .map(|node_run| node_run.node_id.as_str())
                .collect();
            let ready: Vec<&WorkflowGraphNode> = graph
                .ready_set(&completed)
                .into_iter()
                .filter(|node| !in_flight.contains(node.id.as_str()))
                .collect();

            if ready.is_empty() {
                if in_flight.is_empty() {
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

/// Computes the output a control node writes when it completes synchronously.
fn control_node_output(
    graph: &WorkflowGraph,
    node: &WorkflowGraphNode,
    node_runs: &[WorkflowNodeRun],
    context: &ExecutionContext,
) -> String {
    match node.node_type {
        NodeType::Start => context.run.input.clone().unwrap_or_default(),
        NodeType::Output => {
            let parts = graph
                .predecessors(&node.id)
                .iter()
                .map(|predecessor| {
                    node_runs
                        .iter()
                        .find(|node_run| {
                            node_run.node_id == predecessor.id
                                && node_run.status == WorkflowNodeStatus::Succeeded
                        })
                        .map(|node_run| last_assistant_message(node_run.output.as_deref()))
                        .unwrap_or_default()
                })
                .collect::<Vec<String>>();
            parts.join(OUTPUT_PREDECESSOR_SEPARATOR)
        }
        _ => String::new(),
    }
}

/// Computes the run output written at finish: the last output node's output, or the last
/// completed agent's final assistant message when the graph has no output node.
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
    last_agent.map(|node_run| last_assistant_message(node_run.output.as_deref()))
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

/// Extracts the final assistant message from a node conversation array.
fn last_assistant_message(output: Option<&str>) -> String {
    let Some(output) = output else {
        return String::new();
    };
    let Ok(conversation) = serde_json::from_str::<Vec<ConversationEntry>>(output) else {
        return String::new();
    };
    conversation
        .iter()
        .rev()
        .find_map(|entry| (entry.role == "assistant").then_some(entry.text.clone()))
        .unwrap_or_default()
}
