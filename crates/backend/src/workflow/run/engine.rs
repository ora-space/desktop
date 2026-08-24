use super::executor::WorkflowRunNodeExecutor;
use crate::agent_runtime::AgentRuntimeManager;
use crate::clock::SystemClock;
use crate::git_cleanup::KeyedResourceLocks;
use ora_application::{
    FileChange, UuidWorkflowNodeRunIdGenerator, WorkflowGraph, WorkflowRunCallback,
    WorkflowRunControlHandler, WorkflowRunEngine, WorkflowRunEngineRepository,
};
use ora_db::{
    RepositoryPool, SqliteAgentDefinitionRepository, SqliteWorkflowRunEngineRepository,
    SqliteWorkflowRunRepository,
};
use ora_domain::{
    WorkflowNodeRun, WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRunId, WorkflowRunStatus,
};
use ora_logging::{ora_error, ora_warn};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// The concrete run engine as composed by the backend.
pub(crate) type ConcreteWorkflowRunEngine = WorkflowRunEngine<
    SqliteWorkflowRunEngineRepository,
    WorkflowRunNodeExecutor,
    UuidWorkflowNodeRunIdGenerator,
    SystemClock,
>;

/// The concrete control handler exposed to the Web and Tauri adapters.
pub(crate) type ConcreteWorkflowRunControl = WorkflowRunControlHandler<
    SqliteWorkflowRunEngineRepository,
    WorkflowRunNodeExecutor,
    UuidWorkflowNodeRunIdGenerator,
    SystemClock,
    SqliteWorkflowRunRepository,
>;

/// Routes session-driver completions back to the run engine.
///
/// The callback is created before the engine (the engine embeds the executor, which embeds this
/// callback), so the engine reference is attached once the composition root finishes building.
///
/// Every completion and failure is gated by the per-run lock, so a session-driver callback and a
/// manual completion against the same run serialize on the same gate as every other engine entry
/// point. The callback is invoked from a blocking worker (see `WorkflowRunNodeExecutor`), which
/// is what lets it hold the blocking lock across the synchronous engine call.
pub(crate) struct WorkflowRunEngineCallback {
    engine: RwLock<Option<Arc<ConcreteWorkflowRunEngine>>>,
    run_locks: Arc<KeyedResourceLocks>,
}

impl WorkflowRunEngineCallback {
    /// Creates a callback with no engine attached yet.
    fn new(run_locks: Arc<KeyedResourceLocks>) -> Self {
        Self {
            engine: RwLock::new(None),
            run_locks,
        }
    }

    /// Attaches the engine once the composition root has built it.
    fn set_engine(&self, engine: Arc<ConcreteWorkflowRunEngine>) {
        if let Ok(mut guard) = self.engine.write() {
            *guard = Some(engine);
        }
    }
}

impl WorkflowRunCallback for WorkflowRunEngineCallback {
    fn complete_node(
        &self,
        run_id: &WorkflowRunId,
        node_run_id: &WorkflowNodeRunId,
        output: Option<String>,
        stop_reason: Option<String>,
        file_changes: Vec<FileChange>,
    ) {
        // Serialize this completion against every other scheduling-affecting mutation for the run
        // before the synchronous engine call runs.
        let _gate = self.run_locks.acquire_exclusive(run_id.as_ref());
        if let Ok(guard) = self.engine.read()
            && let Some(engine) = guard.as_ref()
            && let Err(error) =
                engine.complete_node(run_id, node_run_id, output, stop_reason, file_changes)
        {
            ora_error!(run_id = %run_id, node_run_id = %node_run_id, error = %error, "node completion callback failed");
        }
    }

    fn fail_node(
        &self,
        run_id: &WorkflowRunId,
        node_run_id: &WorkflowNodeRunId,
        error: String,
        output: Option<String>,
    ) {
        let _gate = self.run_locks.acquire_exclusive(run_id.as_ref());
        if let Ok(guard) = self.engine.read()
            && let Some(engine) = guard.as_ref()
            && let Err(callback_error) = engine.fail_node(node_run_id, error, output)
        {
            ora_error!(run_id = %run_id, node_run_id = %node_run_id, error = %callback_error, "node fail callback failed");
        }
    }
}

/// The run engine control handler and the shared per-run lock, as built by the composition root.
pub(crate) struct WorkflowRunEngineAssembly {
    pub control: Arc<ConcreteWorkflowRunControl>,
    /// Serializes every scheduling-affecting mutation per run. Shared with the callback and the
    /// `Backend` control entry points so no two engine mutations for one run interleave.
    pub run_locks: Arc<KeyedResourceLocks>,
    /// The raw engine, used by boot recovery to resume scheduling on a stalled run.
    pub engine: Arc<ConcreteWorkflowRunEngine>,
}

/// Builds the run engine, its session executor, and control handler.
pub(crate) fn build_workflow_run_engine(
    agent_runtime: Arc<AgentRuntimeManager>,
    pool: RepositoryPool,
    baselines_root: PathBuf,
    clock: SystemClock,
) -> WorkflowRunEngineAssembly {
    let run_locks = KeyedResourceLocks::new();
    let callback = Arc::new(WorkflowRunEngineCallback::new(run_locks.clone()));
    let executor = WorkflowRunNodeExecutor::new(
        agent_runtime,
        pool.clone(),
        SqliteAgentDefinitionRepository::new(pool.clone()),
        callback.clone(),
        clock,
        baselines_root,
    );
    let engine = Arc::new(WorkflowRunEngine::new(
        SqliteWorkflowRunEngineRepository::new(pool.clone()),
        executor,
        UuidWorkflowNodeRunIdGenerator::new(),
        clock,
    ));
    callback.set_engine(engine.clone());
    let control = Arc::new(WorkflowRunControlHandler::new(
        (*engine).clone(),
        Arc::new(SqliteWorkflowRunRepository::new(pool)),
    ));
    WorkflowRunEngineAssembly {
        control,
        run_locks,
        engine,
    }
}

/// Reconciles `Running` runs left by a previous process, after the orphan sweep has failed any run
/// still actively generating.
///
/// A `Pending` node survives only when the frozen graph proves it is an interactive Agent node
/// with a bound session (a genuine awaiting node). Any other `Pending` node is a persistence
/// inconsistency and fails closed rather than being re-dispatched. A `Running` run with neither a
/// running node nor an awaiting node resumes scheduling from persisted state, so a crash between a
/// node completion and its successor scheduling cannot strand the run in `Running`.
pub(crate) fn reconcile_running_workflow_runs(
    engine: &Arc<ConcreteWorkflowRunEngine>,
    run_locks: &Arc<KeyedResourceLocks>,
    pool: &RepositoryPool,
) {
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let run_ids = match repository.list_recoverable_runs() {
        Ok(run_ids) => run_ids,
        Err(error) => {
            ora_error!(error = %error, "workflow run reconcile failed to list recoverable runs");
            return;
        }
    };
    for run_id in run_ids {
        let context = match repository.find_execution_context(&run_id) {
            Ok(Some(context)) => context,
            Ok(None) => continue,
            Err(error) => {
                ora_error!(run_id = %run_id, error = %error, "workflow run reconcile failed to read context");
                continue;
            }
        };
        if context.run.status != WorkflowRunStatus::Running {
            // Terminal runs are not resumed; the orphan sweep already failed any with a live node.
            continue;
        }
        let graph = match WorkflowGraph::parse(&context.graph_json) {
            Ok(graph) => graph,
            Err(error) => {
                ora_error!(run_id = %run_id, error = %error, "workflow run reconcile failed to parse graph");
                continue;
            }
        };
        let node_runs = match repository.list_node_runs(&run_id) {
            Ok(node_runs) => node_runs,
            Err(error) => {
                ora_error!(run_id = %run_id, error = %error, "workflow run reconcile failed to list node runs");
                continue;
            }
        };

        let _gate = run_locks.acquire_exclusive(run_id.as_ref());

        if node_runs
            .iter()
            .any(|node_run| node_run.status == WorkflowNodeStatus::Running)
        {
            // A run still generating was already failed by the orphan sweep; stay inert.
            continue;
        }
        let invalid_pending: Vec<_> = node_runs
            .iter()
            .filter(|node_run| {
                node_run.status == WorkflowNodeStatus::Pending
                    && !is_awaiting_input(node_run, &graph)
            })
            .collect();
        if !invalid_pending.is_empty() {
            for node_run in invalid_pending {
                ora_warn!(run_id = %run_id, node_run_id = %node_run.id, "failing invalid pending node after restart");
                if let Err(error) = engine.fail_node(
                    &node_run.id,
                    "invalid pending node after restart".to_string(),
                    None,
                ) {
                    ora_error!(run_id = %run_id, node_run_id = %node_run.id, error = %error, "failed to fail invalid pending node");
                }
            }
            continue;
        }
        if node_runs
            .iter()
            .any(|node_run| is_awaiting_input(node_run, &graph))
        {
            // A genuine awaiting node: preserve the run for the human to continue.
            continue;
        }
        // No running node and no awaiting node: resume scheduling. A dispatch failure is handled
        // by the normal execution path, which fails the node and run.
        if let Err(error) = engine.resume(&run_id) {
            ora_error!(run_id = %run_id, error = %error, "workflow run reconcile failed to resume scheduling");
        }
    }
}

/// Whether a persisted `Pending` node is a genuine interactive awaiting node, proven by the frozen
/// graph. Any other `Pending` node is a persistence-integrity failure that must fail closed.
fn is_awaiting_input(node_run: &WorkflowNodeRun, graph: &WorkflowGraph) -> bool {
    node_run.status == WorkflowNodeStatus::Pending
        && node_run.session_id.is_some()
        && node_run.node_type == "agent"
        && graph
            .node(&node_run.node_id)
            .and_then(|node| node.agent_config.as_ref())
            .is_some_and(|config| config.interactive)
}

#[cfg(test)]
mod tests {
    use super::is_awaiting_input;
    use ora_application::WorkflowGraph;
    use ora_domain::{
        AuditFields, SessionId, WorkflowNodeRun, WorkflowNodeRunId, WorkflowNodeStatus,
        WorkflowRunId,
    };

    fn node_run(status: WorkflowNodeStatus, session_id: Option<&str>) -> WorkflowNodeRun {
        WorkflowNodeRun::new(
            WorkflowNodeRunId::new("node-1"),
            WorkflowRunId::new("run-1"),
            "a",
            "agent",
            session_id.map(SessionId::new),
            status,
            None,
            None,
            None,
            None,
            Some(1),
            None,
            AuditFields::new(1, 1, false),
        )
    }

    const INTERACTIVE_GRAPH: &str = r#"{"nodes":[{"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"interactive":true,"prompt":"p"}}}],"edges":[]}"#;
    const AUTO_GRAPH: &str = r#"{"nodes":[{"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"p"}}}],"edges":[]}"#;

    /// Only a `Pending` interactive node with a bound session is a genuine awaiting node.
    #[test]
    fn awaiting_input_requires_pending_interactive_and_bound_session() {
        let interactive = WorkflowGraph::parse(INTERACTIVE_GRAPH).unwrap();
        let automatic = WorkflowGraph::parse(AUTO_GRAPH).unwrap();
        let missing = WorkflowGraph::parse(r#"{"nodes":[],"edges":[]}"#).unwrap();

        assert!(is_awaiting_input(
            &node_run(WorkflowNodeStatus::Pending, Some("s")),
            &interactive
        ));
        assert!(!is_awaiting_input(
            &node_run(WorkflowNodeStatus::Running, Some("s")),
            &interactive
        ));
        assert!(!is_awaiting_input(
            &node_run(WorkflowNodeStatus::Pending, None),
            &interactive
        ));
        assert!(!is_awaiting_input(
            &node_run(WorkflowNodeStatus::Pending, Some("s")),
            &automatic
        ));
        assert!(!is_awaiting_input(
            &node_run(WorkflowNodeStatus::Pending, Some("s")),
            &missing
        ));
    }
}
