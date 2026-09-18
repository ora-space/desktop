//! Agent-node execution port and engine-facing errors.
//!
//! These types live outside `engine.rs` so the scheduling core stays under the module-size
//! baseline; the engine still owns dispatch and composite-start checkpointing.

use crate::RepositoryError;
use crate::workflow_run::engine::failure::NodeFailure;
use crate::workflow_run::engine::graph::{GraphError, WorkflowGraph, WorkflowGraphNode};
use crate::workflow_run::engine::node_type::NodeType;
use crate::workflow_run::engine::ports::{ExecutionContext, FileChange};
use crate::workflow_run::engine::variable_pool::WorkflowVariablePool;
use ora_domain::{WorkflowNodeRunId, WorkflowRunId, WorkflowScopeId};
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
        graph: &WorkflowGraph,
        context: &ExecutionContext,
        scope_id: &WorkflowScopeId,
        variable_pool: &WorkflowVariablePool,
    );

    /// Records a pre-node git checkpoint when a composite node-run becomes `Running`.
    ///
    /// Agent nodes already checkpoint inside their session driver. Composite nodes never
    /// dispatch, so the scheduling wave calls this instead; the default is a no-op so
    /// in-memory tests need not take git snapshots.
    fn on_composite_node_started(
        &self,
        _node_run_id: &WorkflowNodeRunId,
        _node: &WorkflowGraphNode,
        _context: &ExecutionContext,
    ) {
    }
}

/// Shares one executor between the Agent runtime and composite-start checkpointing.
pub(super) struct SharedNodeExecutor(pub(super) Arc<dyn NodeExecutor>);

impl NodeExecutor for SharedNodeExecutor {
    fn dispatch(
        &self,
        node_run_id: &WorkflowNodeRunId,
        node: &WorkflowGraphNode,
        graph: &WorkflowGraph,
        context: &ExecutionContext,
        scope_id: &WorkflowScopeId,
        variable_pool: &WorkflowVariablePool,
    ) {
        self.0
            .dispatch(node_run_id, node, graph, context, scope_id, variable_pool);
    }

    fn on_composite_node_started(
        &self,
        node_run_id: &WorkflowNodeRunId,
        node: &WorkflowGraphNode,
        context: &ExecutionContext,
    ) {
        self.0.on_composite_node_started(node_run_id, node, context);
    }
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

    /// Reports a failed node execution with a classified failure and any accumulated output.
    fn fail_node(
        &self,
        run_id: &WorkflowRunId,
        node_run_id: &WorkflowNodeRunId,
        failure: NodeFailure,
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
    #[error("workflow Loop state cannot be serialized: {message}")]
    LoopState { message: String },
}
