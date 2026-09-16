//! Node runtimes: one node type's execution strategy, resolved through a registry.
//!
//! The scheduling core stays free of node-type branching (ADR "node runtime orchestration" D1):
//! every node type registers a runtime here, and `run_schedule` looks a runtime up by node type
//! and dispatches on the registered execution form — never on the node type itself. Adding a
//! node type means adding a runtime plus one registration line; the engine core does not change.

mod control;
mod iteration;

use crate::workflow_run::engine::graph::{WorkflowGraph, WorkflowGraphNode};
use crate::workflow_run::engine::iteration::RoundOutcome;
use crate::workflow_run::engine::node_type::NodeType;
use crate::workflow_run::engine::ports::ExecutionContext;
use crate::workflow_run::engine::variable_pool::WorkflowVariablePool;
use control::{ConditionRuntime, OutputRuntime, StartRuntime};
use iteration::IterationRuntime;
use ora_domain::{WorkflowNodeRun, WorkflowNodeRunId, WorkflowNodeStatus};
use std::cmp::Reverse;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

/// Base capability shared by every node runtime regardless of execution form.
///
/// A runtime owns one node type's execution policy: how a node-run of that type is started and
/// driven to a terminal state. The engine core owns only scheduling — when to start, when to
/// finish — and consults this trait for the type-specific parts.
pub trait NodeRuntime: Send + Sync {
    /// Computes the scalar input recorded on a node-run when a scheduling wave starts it.
    fn start_input(&self, node: &WorkflowGraphNode, context: &ExecutionContext) -> Option<String>;

    /// The precedence rank with which this runtime's succeeded node-runs contribute the run's
    /// final output; `None` means nodes of this type never contribute.
    ///
    /// Run-output selection is node-type policy, so it is declared here as runtime metadata
    /// (ADR "node runtime orchestration" D1) instead of a type list in the scheduling core:
    /// the run output is the output of the latest-finished succeeded node-run carrying the
    /// lowest rank present in the run. A new terminal node type decides its own precedence by
    /// declaring a rank, without editing any scheduling-layer code.
    fn run_output_rank(&self) -> Option<u32>;
}

/// A runtime that completes its nodes synchronously inside a scheduling wave.
///
/// The call happens while the per-run serial gate is held, so implementations must be pure: no
/// IO, no waiting, bounded work (ADR D2). The signature upholds this by construction — a swift
/// runtime receives only in-memory committed facts, so no repository or async handle is ever
/// handed in — but Rust cannot stop an implementation from reaching `std::fs` or sleeping on
/// its own, so the discipline is also pinned by an executable source constraint:
/// `node_runtime_module_performs_no_io_or_waiting` (see `engine/tests.rs`) rejects IO and
/// waiting primitives anywhere in the runtime module.
pub trait SwiftNodeRuntime: NodeRuntime {
    /// Computes the terminal output of one running node, or the failure message that fails it.
    fn complete_running(
        &self,
        node: &WorkflowGraphNode,
        completion: &SwiftCompletion<'_>,
    ) -> Result<String, String>;
}

/// A runtime whose nodes are driven in the background after dispatch.
///
/// `dispatch` must return immediately; the node's terminal state is reported later through
/// [`WorkflowRunCallback`](crate::workflow_run::engine::WorkflowRunCallback), which re-enters
/// the engine under the same per-run serial gate as every other state transition.
pub trait AsyncNodeRuntime: NodeRuntime {
    /// Starts background driving of one node-run the wave just started.
    fn dispatch(
        &self,
        node_run_id: &WorkflowNodeRunId,
        node: &WorkflowGraphNode,
        context: &ExecutionContext,
    );
}

/// The next transition one Running composite node-run needs, computed purely from persisted
/// facts (ADR "node runtime orchestration" D2/D5; iteration ADR D2 advance).
///
/// Planning is pure: the engine executes the plan through repository transactions, so the
/// runtime itself performs no IO under the run lock.
#[derive(Debug, Clone, PartialEq)]
pub enum CompositeAdvancePlan {
    /// Nothing to do: the current round is in flight or the node is not driven by advance.
    Noop,
    /// Bind a fresh round's `item`/`index` and start its first ready members.
    StartRound {
        round: u32,
        item: serde_json::Value,
        node_ids: Vec<String>,
    },
    /// Start newly ready members of the current round (no pool write).
    StartRegionNodes { node_ids: Vec<String> },
    /// Settle the drained round and continue atomically.
    SettleRound {
        round: u32,
        entry: RoundOutcome,
        continuation: CompositeContinuation,
    },
    /// Complete the composite node without settling any round (empty iterator source).
    CompleteNode {
        exposed: Vec<(String, serde_json::Value)>,
        output: Option<String>,
    },
    /// Fail the composite node; its own failures always propagate to the run.
    FailNode { error: String },
}

/// How one settled iteration round continues, at the planning level (pure node ids; the engine
/// materializes rows before executing).
#[derive(Debug, Clone, PartialEq)]
pub enum CompositeContinuation {
    /// Start the next round: bind its `item`/`index` and start its first ready members.
    StartNextRound {
        round: u32,
        item: serde_json::Value,
        node_ids: Vec<String>,
    },
    /// Complete the composite node, writing the ledger-derived exposed variables.
    Complete {
        exposed: Vec<(String, serde_json::Value)>,
        output: Option<String>,
    },
    /// Fail the composite node (and its run) — used when a `fail`-strategy round fails.
    Fail { error: String },
}

/// A runtime that owns a region and drives it over multiple rounds.
///
/// The engine hands every Running composite node-run back to its runtime on each scheduling
/// wave (`advance`); the runtime answers with a [`CompositeAdvancePlan`] derived purely from
/// the persisted rows, ledger, and variable pool, so a restart replays to the same point
/// without any in-memory state.
pub trait CompositeNodeRuntime: NodeRuntime {
    /// Computes the next transition for one Running composite node-run.
    ///
    /// `Err` carries a failure message that fails the node (and the run): composite-own
    /// failures such as a non-array iterator source or an exceeded safety ceiling.
    fn plan_advance(
        &self,
        node: &WorkflowGraphNode,
        graph: &WorkflowGraph,
        node_runs: &[WorkflowNodeRun],
        payload: &crate::workflow_run::engine::skill_delivery::WorkflowRunPayload,
    ) -> Result<CompositeAdvancePlan, String>;
}

/// The committed facts a swift runtime reads to complete one running node inside a wave.
///
/// Everything here is in-memory state loaded by the scheduling wave; the runtime module holds
/// no IO or persistence handles at all — the no-IO-under-the-run-lock invariant (ADR D2)
/// depends on that boundary, not on the signature alone.
pub struct SwiftCompletion<'a> {
    /// The running node-run being completed.
    pub node_run: &'a WorkflowNodeRun,
    /// The run's kickoff input, which the Start node records as its output.
    pub run_input: Option<&'a str>,
    /// The committed variable pool the node reads.
    pub pool: &'a WorkflowVariablePool,
    /// Every node-run row of the run as loaded before this wave's completions, for structural
    /// checks such as Output uniqueness.
    pub node_runs: &'a [WorkflowNodeRun],
}

/// One registered node runtime, tagged by its execution form.
///
/// The tag keeps illegal dispatch unrepresentable: the wave can only ask a swift runtime to
/// complete synchronously, only ask an async runtime to dispatch to its background driver, and
/// only ask a composite runtime to plan its next advance.
#[derive(Clone)]
pub enum RegisteredNodeRuntime {
    /// Completes synchronously inside the scheduling wave.
    Swift(Arc<dyn SwiftNodeRuntime>),
    /// Dispatched to a background driver that reports through the callback sink.
    Async(Arc<dyn AsyncNodeRuntime>),
    /// Drives a region over rounds; the engine re-plans it on every scheduling wave.
    Composite(Arc<dyn CompositeNodeRuntime>),
}

impl RegisteredNodeRuntime {
    /// The run-output precedence rank of the registered runtime's node type.
    fn run_output_rank(&self) -> Option<u32> {
        match self {
            Self::Swift(runtime) => runtime.run_output_rank(),
            Self::Async(runtime) => runtime.run_output_rank(),
            Self::Composite(runtime) => runtime.run_output_rank(),
        }
    }
}

/// Maps node types onto their registered runtimes.
///
/// The registry is the single place a node type couples to an execution strategy: the
/// scheduling core looks a runtime up by node type and dispatches on the registered form,
/// never on the node type itself.
#[derive(Clone, Default)]
pub struct NodeRuntimeRegistry {
    runtimes: HashMap<NodeType, RegisteredNodeRuntime>,
}

impl NodeRuntimeRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers one node type's runtime, replacing any previous registration.
    pub fn register(&mut self, node_type: NodeType, runtime: RegisteredNodeRuntime) {
        self.runtimes.insert(node_type, runtime);
    }

    /// Looks up the registered runtime of one node type.
    pub fn runtime(&self, node_type: NodeType) -> Option<&RegisteredNodeRuntime> {
        self.runtimes.get(&node_type)
    }

    /// Computes the scalar input a node-run of this type records when the wave starts it.
    ///
    /// A node type with no registered runtime records no input, matching how graphs that were
    /// rejected at start cannot reach scheduling.
    pub fn start_input(
        &self,
        node: &WorkflowGraphNode,
        context: &ExecutionContext,
    ) -> Option<String> {
        match self.runtime(node.node_type) {
            Some(RegisteredNodeRuntime::Swift(runtime)) => runtime.start_input(node, context),
            Some(RegisteredNodeRuntime::Async(runtime)) => runtime.start_input(node, context),
            Some(RegisteredNodeRuntime::Composite(runtime)) => runtime.start_input(node, context),
            None => None,
        }
    }

    /// Computes the run output written at finish, from the run-output precedence the registered
    /// runtimes declare (ADR "node runtime orchestration" D1).
    ///
    /// The winning candidate is the succeeded node-run whose runtime declares the lowest
    /// `run_output_rank`; ties on a rank are broken by the latest `finished_at`, then by row
    /// order. A succeeded node-run whose type has no runtime, or whose runtime declares no
    /// rank, never contributes.
    pub fn compute_run_output(&self, node_runs: &[WorkflowNodeRun]) -> Option<String> {
        node_runs
            .iter()
            // Region rows are per-round members of a composite; only outer rows contribute the
            // run's final output (ADR "iteration composite runtime" Stage B).
            .filter(|node_run| node_run.iteration.is_none())
            .filter(|node_run| node_run.status == WorkflowNodeStatus::Succeeded)
            .filter_map(|node_run| {
                let rank = NodeType::from_str(&node_run.node_type)
                    .ok()
                    .and_then(|node_type| self.runtime(node_type))
                    .and_then(RegisteredNodeRuntime::run_output_rank)?;
                Some((rank, node_run))
            })
            // Max over (reversed rank, finish time) picks the lowest rank first and the latest
            // finish within it, preferring the later row on full ties - matching the previous
            // latest-by-type selection exactly.
            .max_by_key(|(rank, node_run)| (Reverse(*rank), node_run.finished_at.unwrap_or(0)))
            .and_then(|(_, node_run)| node_run.output.clone())
    }
}

/// Builds the standard runtime set: the swift Start/Condition/Output runtimes and the async
/// Agent runtime wrapping the session executor.
///
/// This is the registry assembly point — the one place node types map to runtimes. Registering
/// a future node type (aggregator, code, HTTP, remote subworkflow) is one line here plus the
/// runtime implementation, without editing the engine core.
pub fn standard_node_runtimes<E>(agent_executor: E) -> NodeRuntimeRegistry
where
    E: super::engine::NodeExecutor + 'static,
{
    let mut runtimes = NodeRuntimeRegistry::new();
    runtimes.register(
        NodeType::Start,
        RegisteredNodeRuntime::Swift(Arc::new(StartRuntime)),
    );
    runtimes.register(
        NodeType::Condition,
        RegisteredNodeRuntime::Swift(Arc::new(ConditionRuntime)),
    );
    runtimes.register(
        NodeType::Output,
        RegisteredNodeRuntime::Swift(Arc::new(OutputRuntime)),
    );
    runtimes.register(
        NodeType::Agent,
        RegisteredNodeRuntime::Async(Arc::new(AgentNodeRuntime::new(agent_executor))),
    );
    runtimes.register(
        NodeType::Iteration,
        RegisteredNodeRuntime::Composite(Arc::new(IterationRuntime)),
    );
    runtimes
}

/// The Agent node runtime: dispatches each started node-run to the session executor.
///
/// The wrapped executor drives one dedicated Ora session per node in the background and
/// reports the terminal state through [`WorkflowRunCallback`](crate::workflow_run::engine::WorkflowRunCallback).
struct AgentNodeRuntime<E> {
    executor: E,
}

impl<E> AgentNodeRuntime<E> {
    /// Wraps one session executor as the Agent runtime.
    fn new(executor: E) -> Self {
        Self { executor }
    }
}

impl<E> NodeRuntime for AgentNodeRuntime<E>
where
    E: super::engine::NodeExecutor,
{
    /// An agent node-run records its prompt template as the scalar input.
    fn start_input(&self, node: &WorkflowGraphNode, _context: &ExecutionContext) -> Option<String> {
        node.agent_config
            .as_ref()
            .map(|config| config.prompt.clone())
    }

    /// A completed Agent is the run-output fallback: it contributes only when no higher-ranked
    /// terminal runtime (Output) completed.
    fn run_output_rank(&self) -> Option<u32> {
        Some(1)
    }
}

impl<E> AsyncNodeRuntime for AgentNodeRuntime<E>
where
    E: super::engine::NodeExecutor,
{
    fn dispatch(
        &self,
        node_run_id: &WorkflowNodeRunId,
        node: &WorkflowGraphNode,
        context: &ExecutionContext,
    ) {
        self.executor.dispatch(node_run_id, node, context);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_run::engine::engine::NodeExecutor;
    use crate::workflow_run::engine::graph::WorkflowGraph;
    use crate::workflow_run::engine::ports::ExecutionContext;
    use ora_domain::{
        AuditFields, WorkflowId, WorkflowNodeRun, WorkflowNodeRunId, WorkflowRun, WorkflowRunId,
        WorkflowRunStatus, WorkflowSnapshotId, Workspace, WorkspaceId,
    };
    use ora_domain::{ProjectId, WorkspaceKind, WorkspaceLifecycle, WorkspaceLocation};
    use pretty_assertions::assert_eq;

    /// An executor that never dispatches, for tests that assemble the standard registry.
    struct NoopExecutor;

    impl NodeExecutor for NoopExecutor {
        fn dispatch(
            &self,
            _node_run_id: &WorkflowNodeRunId,
            _node: &WorkflowGraphNode,
            _context: &ExecutionContext,
        ) {
        }
    }

    /// Builds an execution context whose run carries the given kickoff input; workspace and
    /// graph are unused by these tests.
    fn execution_context_with_input(input: Option<&str>) -> ExecutionContext {
        ExecutionContext {
            run: WorkflowRun::new(
                WorkflowRunId::new("run-1"),
                WorkspaceId::new("workspace-1"),
                WorkflowId::new("workflow-1"),
                WorkflowSnapshotId::new("snapshot-1"),
                "Review",
                WorkflowRunStatus::Running,
                None,
                input.map(str::to_string),
                None,
                None,
                None,
                None,
                None,
                AuditFields::new(1, 1, false),
            ),
            workspace: Workspace::new(
                WorkspaceId::new("workspace-1"),
                ProjectId::new("project-1"),
                WorkspaceKind::Main,
                WorkspaceLocation::local_filesystem("/tmp/workspace"),
                WorkspaceLifecycle::Active,
                AuditFields::new(1, 1, false),
            ),
            graph_json: String::new(),
        }
    }

    /// A node-run row fixture for run-output selection.
    fn node_run(
        node_id: &str,
        node_type: &str,
        status: WorkflowNodeStatus,
        output: Option<&str>,
        finished_at: Option<i64>,
    ) -> WorkflowNodeRun {
        WorkflowNodeRun::new(
            WorkflowNodeRunId::new(format!("node-run-{node_id}")),
            WorkflowRunId::new("run-1"),
            node_id,
            node_type,
            None,
            status,
            None,
            output.map(str::to_string),
            None,
            None,
            Some(1),
            finished_at,
            AuditFields::new(1, 1, false),
        )
    }

    /// The standard registry registers a runtime for every v1-supported node type, and only
    /// for those; recognized-but-unsupported types resolve to nothing.
    #[test]
    fn standard_registry_covers_exactly_the_supported_node_types() {
        let runtimes = standard_node_runtimes(NoopExecutor);
        for node_type in [NodeType::Start, NodeType::Condition, NodeType::Output] {
            assert!(
                matches!(
                    runtimes.runtime(node_type),
                    Some(RegisteredNodeRuntime::Swift(_))
                ),
                "{node_type} must be a swift runtime"
            );
        }
        assert!(matches!(
            runtimes.runtime(NodeType::Agent),
            Some(RegisteredNodeRuntime::Async(_))
        ));
        for node_type in [NodeType::Prompt, NodeType::Tool] {
            assert!(runtimes.runtime(node_type).is_none());
        }
    }

    /// A node-run records the input its runtime declares: the kickoff input for Start, the
    /// prompt for Agent, and nothing for control nodes.
    #[test]
    fn start_input_follows_the_registered_runtime() {
        let graph = WorkflowGraph::parse(
            r#"{"nodes":[
                {"id":"start","data":{"kind":"start"}},
                {"id":"agent","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"do"}}},
                {"id":"c","data":{"kind":"condition"}},
                {"id":"out","data":{"kind":"output"}}
            ],"edges":[]}"#,
        )
        .unwrap();
        let context = execution_context_with_input(Some("kickoff"));
        let runtimes = standard_node_runtimes(NoopExecutor);
        assert_eq!(
            runtimes.start_input(graph.node("start").unwrap(), &context),
            Some("kickoff".to_string())
        );
        assert_eq!(
            runtimes.start_input(graph.node("agent").unwrap(), &context),
            Some("do".to_string())
        );
        assert_eq!(
            runtimes.start_input(graph.node("c").unwrap(), &context),
            None
        );
        assert_eq!(
            runtimes.start_input(graph.node("out").unwrap(), &context),
            None
        );
    }

    /// The run output prefers the latest finished Output node and only falls back to the
    /// latest finished Agent when no Output completed, exactly as the runtimes' declared
    /// run-output ranks prescribe.
    #[test]
    fn run_output_prefers_output_nodes_and_falls_back_to_agents() {
        let runtimes = standard_node_runtimes(NoopExecutor);
        let node_runs = vec![
            node_run(
                "a",
                "agent",
                WorkflowNodeStatus::Succeeded,
                Some("agent output"),
                Some(10),
            ),
            node_run(
                "out-1",
                "output",
                WorkflowNodeStatus::Succeeded,
                Some("first"),
                Some(20),
            ),
            node_run(
                "out-2",
                "output",
                WorkflowNodeStatus::Succeeded,
                Some("last"),
                Some(30),
            ),
        ];
        assert_eq!(
            runtimes.compute_run_output(&node_runs),
            Some("last".to_string())
        );

        let agents_only = vec![
            node_run(
                "a",
                "agent",
                WorkflowNodeStatus::Succeeded,
                Some("older"),
                Some(10),
            ),
            node_run(
                "b",
                "agent",
                WorkflowNodeStatus::Succeeded,
                Some("newer"),
                Some(20),
            ),
        ];
        assert_eq!(
            runtimes.compute_run_output(&agents_only),
            Some("newer".to_string())
        );

        // A succeeded Output with no recorded output still wins over later Agent output.
        let empty_output = vec![
            node_run(
                "a",
                "agent",
                WorkflowNodeStatus::Succeeded,
                Some("agent output"),
                Some(30),
            ),
            node_run(
                "out",
                "output",
                WorkflowNodeStatus::Succeeded,
                None,
                Some(20),
            ),
        ];
        assert_eq!(runtimes.compute_run_output(&empty_output), None);

        // Failed and cancelled runs never contribute.
        let no_contributors = vec![
            node_run(
                "a",
                "agent",
                WorkflowNodeStatus::Failed,
                Some("failed"),
                Some(10),
            ),
            node_run(
                "out",
                "output",
                WorkflowNodeStatus::Cancelled,
                Some("cancelled"),
                Some(20),
            ),
        ];
        assert_eq!(runtimes.compute_run_output(&no_contributors), None);
    }

    /// A node-run row never carries a session binding for swift types in this fixture helper;
    /// asserted here so the fixture stays honest for the runtime tests that use it.
    #[test]
    fn node_run_fixture_binds_no_session() {
        let run = node_run("out", "output", WorkflowNodeStatus::Succeeded, None, None);
        assert_eq!(run.session_id, None);
        assert_eq!(run.status, WorkflowNodeStatus::Succeeded);
    }
}
