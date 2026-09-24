use super::executor::WorkflowRunNodeExecutor;
use super::retry_timer::WorkflowRetryTimers;
use super::transitions::WorkflowRunTransitions;
use crate::agent_runtime::AgentRuntimeManager;
use crate::app_event::AppEventPublisher;
use crate::clock::SystemClock;
use crate::git_cleanup::KeyedResourceLocks;
use ora_application::{
    FileChange, NodeFailure, NodeType, UuidWorkflowNodeRunIdGenerator, WorkflowGraph,
    WorkflowRunCallback, WorkflowRunControlHandler, WorkflowRunEngine, WorkflowRunEngineRepository,
    WorkflowRunInvalidationPublisher,
};
use ora_contracts::AppEvent;
use ora_db::{
    RepositoryPool, SqliteAgentDefinitionRepository, SqliteWorkflowRunEngineRepository,
    SqliteWorkflowRunRepository,
};
use ora_domain::{
    WorkflowNodeRun, WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRunId, WorkflowRunStatus,
};
use ora_logging::{ora_error, ora_warn};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, RwLock};

/// The concrete run engine as composed by the backend.
pub(crate) type ConcreteWorkflowRunEngine = WorkflowRunEngine<
    SqliteWorkflowRunEngineRepository,
    UuidWorkflowNodeRunIdGenerator,
    SystemClock,
>;

/// The concrete control handler exposed to the Web and Tauri adapters.
pub(crate) type ConcreteWorkflowRunControl = WorkflowRunControlHandler<
    SqliteWorkflowRunEngineRepository,
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
        structured_output: Option<serde_json::Value>,
        stop_reason: Option<String>,
        file_changes: Vec<FileChange>,
    ) {
        // Serialize this completion against every other scheduling-affecting mutation for the run
        // before the synchronous engine call runs.
        let _gate = self.run_locks.acquire_exclusive(run_id.as_ref());
        if let Ok(guard) = self.engine.read()
            && let Some(engine) = guard.as_ref()
            && let Err(error) = engine.complete_node(
                run_id,
                node_run_id,
                output,
                structured_output,
                stop_reason,
                file_changes,
            )
        {
            ora_error!(run_id = %run_id, node_run_id = %node_run_id, error = %error, "node completion callback failed");
        }
    }

    fn fail_node(
        &self,
        run_id: &WorkflowRunId,
        node_run_id: &WorkflowNodeRunId,
        failure: NodeFailure,
    ) {
        let _gate = self.run_locks.acquire_exclusive(run_id.as_ref());
        if let Ok(guard) = self.engine.read()
            && let Some(engine) = guard.as_ref()
            && let Err(callback_error) = engine.fail_node(run_id, node_run_id, failure)
        {
            ora_error!(run_id = %run_id, node_run_id = %node_run_id, error = %callback_error, "node fail callback failed");
        }
    }
}

/// The run engine control handler and the shared per-run lock, as built by the composition root.
pub(crate) struct WorkflowRunEngineAssembly {
    pub control: Arc<ConcreteWorkflowRunControl>,
    /// Serializes every scheduling-affecting mutation per run. Shared with the callback and the
    /// workflow-run control entry points so no two engine mutations for one run interleave.
    pub run_locks: Arc<KeyedResourceLocks>,
    /// The raw engine, used by boot recovery to resume scheduling on a stalled run.
    pub engine: Arc<ConcreteWorkflowRunEngine>,
    /// The commit-and-publish sink for node-run transitions committed outside the engine (the
    /// interactive chain), sharing the engine's invalidation mechanism.
    pub transitions: Arc<WorkflowRunTransitions>,
}

/// Projects engine run invalidations onto the shared application event stream.
///
/// The engine publishes one invalidation after every committed run or node-run state
/// transition (ADR "node runtime orchestration" D7); the event identifies only which run's
/// persisted state changed, and observers re-query the authoritative persistence.
pub(crate) struct WorkflowRunInvalidations {
    events: AppEventPublisher,
}

impl WorkflowRunInvalidations {
    /// Bridges the engine's invalidation port onto the application event hub.
    fn new(events: AppEventPublisher) -> Self {
        Self { events }
    }
}

impl WorkflowRunInvalidationPublisher for WorkflowRunInvalidations {
    fn publish_run_invalidated(&self, run_id: &WorkflowRunId) {
        // Publishing is best-effort and non-blocking: a dropped event only leaves a stale view
        // that the next transition or refresh clears.
        self.events.try_publish(AppEvent::WorkflowRunInvalidated {
            run_id: run_id.to_string(),
        });
    }
}

/// Builds the run engine, its session executor, and control handler.
pub(crate) fn build_workflow_run_engine(
    agent_runtime: Arc<AgentRuntimeManager>,
    pool: RepositoryPool,
    baselines_root: PathBuf,
    clock: SystemClock,
    app_events: AppEventPublisher,
) -> WorkflowRunEngineAssembly {
    let run_locks = KeyedResourceLocks::new();
    let callback = Arc::new(WorkflowRunEngineCallback::new(run_locks.clone()));
    // One invalidation mechanism serves every commit site: the engine publishes its own
    // transitions, and the interactive chain's direct commits (parking an awaiting node,
    // human turns beginning and ending) publish through the same bridge via the transitions
    // sink (ADR "node runtime orchestration" D7).
    let invalidations = Arc::new(WorkflowRunInvalidations::new(app_events));
    let transitions = Arc::new(WorkflowRunTransitions::new(
        pool.clone(),
        invalidations.clone(),
    ));
    let executor = WorkflowRunNodeExecutor::new(
        agent_runtime,
        pool.clone(),
        SqliteAgentDefinitionRepository::new(pool.clone()),
        callback.clone(),
        clock,
        baselines_root,
        transitions.clone(),
    );
    let retry_timers = Arc::new(WorkflowRetryTimers::new(run_locks.clone(), clock));
    let engine = Arc::new(
        WorkflowRunEngine::with_run_events(
            SqliteWorkflowRunEngineRepository::new(pool.clone()),
            executor,
            UuidWorkflowNodeRunIdGenerator::new(),
            clock,
            invalidations,
        )
        .with_retry_timer(retry_timers.clone()),
    );
    callback.set_engine(engine.clone());
    retry_timers.set_engine(&engine);
    let control = Arc::new(WorkflowRunControlHandler::new(
        (*engine).clone(),
        Arc::new(SqliteWorkflowRunRepository::new(pool)),
    ));
    WorkflowRunEngineAssembly {
        control,
        run_locks,
        engine,
        transitions,
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

        if node_runs.iter().any(|node_run| {
            node_run.status == WorkflowNodeStatus::Running
                && NodeType::from_str(&node_run.node_type)
                    .map(|node_type| !node_type.is_composite())
                    .unwrap_or(true)
        }) {
            // A run still generating a non-composite row was already failed by the orphan
            // sweep; stay inert. A `Running` composite row is not generating — its runtime
            // re-plans from persisted facts, so the run resumes below (ADR "iteration
            // composite runtime" D2).
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
                    &run_id,
                    &node_run.id,
                    ora_application::NodeFailure::new(
                        ora_application::NodeFailureKind::InterruptedByRestart,
                        "invalid pending node after restart",
                    ),
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
            .execution_node(&node_run.node_id)
            .and_then(|node| node.agent_config.as_ref())
            .is_some_and(|config| config.interactive)
}

#[cfg(test)]
mod tests {
    use super::super::test_fixture::{
        CONDITION_GRAPH, CONTROL_GRAPH, ClockAt, NoopExecutor, RecordingInvalidations, SeqGen,
        TWO_AGENT_GRAPH, bootstrap, run_test, seeded_pending_run, started_run,
    };
    use super::{WorkflowRunInvalidations, is_awaiting_input};
    use crate::app_event::AppEventHub;
    use ora_application::{
        UuidWorkflowNodeRunIdGenerator, WorkflowGraph, WorkflowRunEngine,
        WorkflowRunEngineRepository, WorkflowRunInvalidationPublisher, WorkflowRunRepository,
    };
    use ora_contracts::AppEvent;
    use ora_db::{SqliteWorkflowRunEngineRepository, SqliteWorkflowRunRepository};
    use ora_domain::{
        AuditFields, SessionId, WorkflowNodeRun, WorkflowNodeRunId, WorkflowNodeStatus,
        WorkflowRunId, WorkflowRunStatus,
    };
    use pretty_assertions::assert_eq;
    use std::sync::Arc;

    fn node_run(status: WorkflowNodeStatus, session_id: Option<&str>) -> WorkflowNodeRun {
        WorkflowNodeRun::new(
            WorkflowNodeRunId::new("node-1"),
            WorkflowRunId::new("run-1"),
            ora_domain::WorkflowScopeId::new("root:run-1"),
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

    const LOOP_GRAPH: &str = r#"{
        "schemaVersion":2,
        "nodes":[
            {"id":"start","data":{"kind":"start"}},
            {"id":"loop","data":{"kind":"loop","loopConfig":{
                "maxIterations":3,
                "variables":[{"name":"draft","valueType":"string","initial":{"kind":"constant","value":""},"feedback":["writer","output"]}],
                "until":{"logic":"and","conditions":[{"variableSelector":["writer","output"],"operator":"equals","value":"done"}]},
                "outputs":[{"name":"result","variableSelector":["writer","output"]}]
            }}},
            {"id":"out","data":{"kind":"output","outputs":[{"name":"result","variableSelector":["loop","result"]}]}},
            {"id":"entry","parentId":"loop","data":{"kind":"start","containerId":"loop"}},
            {"id":"writer","data":{"kind":"agent","containerId":"loop","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"Revise {{#loop.draft#}}"}}}
        ],
        "edges":[
            {"source":"start","target":"loop"},
            {"source":"loop","target":"out"},
            {"source":"entry","target":"writer"}
        ]
    }"#;

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

    /// The production repository and scheduler execute isolated rounds and publish only final output.
    #[test]
    fn loop_runs_until_the_condition_exports_its_result() {
        let (temp, pool) = bootstrap();
        let (run_id, first_nodes) = started_run(&temp, &pool, LOOP_GRAPH);
        let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
        let first_writer = first_nodes
            .iter()
            .find(|node| node.node_id == "writer")
            .unwrap();
        assert_eq!(first_writer.status, WorkflowNodeStatus::Running);

        let engine = WorkflowRunEngine::new(
            repository.clone(),
            NoopExecutor,
            UuidWorkflowNodeRunIdGenerator,
            ClockAt(50),
        );
        repository
            .complete_node(
                &first_writer.id,
                Some("again".into()),
                None,
                None,
                vec![],
                45,
            )
            .unwrap();
        // Simulate a process stop after the child commit but before its scheduling callback.
        engine.resume(&run_id).unwrap();
        let second_writer = repository
            .list_node_runs(&run_id)
            .unwrap()
            .into_iter()
            .find(|node| node.node_id == "writer" && node.status == WorkflowNodeStatus::Running)
            .unwrap();
        assert_ne!(second_writer.scope_id, first_writer.scope_id);

        engine
            .complete_node(
                &run_id,
                &second_writer.id,
                Some("done".into()),
                None,
                None,
                vec![],
            )
            .unwrap();
        let run = SqliteWorkflowRunRepository::new(pool.clone())
            .find_run(&run_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            (run.status, run.output),
            (
                WorkflowRunStatus::Succeeded,
                Some(r#"{"result":"done"}"#.into())
            )
        );
        let all_nodes = repository.list_node_runs(&run_id).unwrap();
        let loop_parent = all_nodes
            .iter()
            .find(|node| node.node_id == "loop")
            .unwrap();
        assert_eq!(
            repository.find_active_loop_round(&loop_parent.id).unwrap(),
            None
        );
        assert_eq!(
            all_nodes
                .iter()
                .filter(|node| {
                    node.node_id == "writer" && node.status == WorkflowNodeStatus::Succeeded
                })
                .count(),
            2
        );
    }

    /// A false condition on the final permitted round terminalizes the scope, parent, and run.
    #[test]
    fn loop_iteration_limit_fails_the_run() {
        let (temp, pool) = bootstrap();
        let graph = LOOP_GRAPH.replacen("\"maxIterations\":3", "\"maxIterations\":1", 1);
        let (run_id, nodes) = started_run(&temp, &pool, &graph);
        let writer = nodes.iter().find(|node| node.node_id == "writer").unwrap();
        let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
        let engine = WorkflowRunEngine::new(
            repository.clone(),
            NoopExecutor,
            UuidWorkflowNodeRunIdGenerator,
            ClockAt(50),
        );

        engine
            .complete_node(
                &run_id,
                &writer.id,
                Some("again".into()),
                None,
                None,
                vec![],
            )
            .unwrap();

        let run = SqliteWorkflowRunRepository::new(pool)
            .find_run(&run_id)
            .unwrap()
            .unwrap();
        assert_eq!(run.status, WorkflowRunStatus::Failed);
        assert!(
            run.error
                .as_deref()
                .is_some_and(|error| error.contains("did not terminate within 1 rounds"))
        );
        let parent = repository
            .list_node_runs(&run_id)
            .unwrap()
            .into_iter()
            .find(|node| node.node_id == "loop")
            .unwrap();
        assert_eq!(parent.status, WorkflowNodeStatus::Failed);
        assert_eq!(repository.find_active_loop_round(&parent.id).unwrap(), None);
    }

    /// Cancelling a Loop run releases the active round and every child node together.
    #[test]
    fn cancelling_loop_settles_its_active_round() {
        let (temp, pool) = bootstrap();
        let (run_id, nodes) = started_run(&temp, &pool, LOOP_GRAPH);
        let parent = nodes.iter().find(|node| node.node_id == "loop").unwrap();
        let repository = SqliteWorkflowRunEngineRepository::new(pool);
        let engine = WorkflowRunEngine::new(
            repository.clone(),
            NoopExecutor,
            UuidWorkflowNodeRunIdGenerator,
            ClockAt(50),
        );

        assert_eq!(
            engine.cancel(&run_id).unwrap(),
            ora_application::CancelWorkflowRunResult::Cancelled
        );
        assert_eq!(repository.find_active_loop_round(&parent.id).unwrap(), None);
        assert!(
            repository
                .list_node_runs(&run_id)
                .unwrap()
                .iter()
                .filter(|node| node.status == WorkflowNodeStatus::Cancelled)
                .count()
                >= 2
        );
    }

    /// An engine assembled without an event bus drops every invalidation, so engines built by
    /// `new` (tests, tools) never observe events while remaining behavior-identical.
    #[test]
    fn engines_without_a_publisher_drop_invalidations() {
        run_test(async {
            let (temp, pool) = bootstrap();
            let run_id = seeded_pending_run(&temp, &pool, CONTROL_GRAPH);
            let engine = WorkflowRunEngine::new(
                SqliteWorkflowRunEngineRepository::new(pool.clone()),
                NoopExecutor,
                SeqGen::default(),
                ClockAt(40),
            );
            engine.start(&run_id).unwrap();
            let run = SqliteWorkflowRunRepository::new(pool)
                .find_run(&run_id)
                .unwrap()
                .unwrap();
            assert_eq!(run.status, WorkflowRunStatus::Succeeded);
        });
    }

    /// Every committed state transition of a scheduling wave publishes one invalidation that
    /// carries only the run id (ADR D7): the run start, each swift completion, each started
    /// wave, and the run finish.
    #[test]
    fn scheduling_waves_publish_one_invalidation_per_committed_transition() {
        run_test(async {
            let (temp, pool) = bootstrap();
            let run_id = seeded_pending_run(&temp, &pool, CONTROL_GRAPH);
            let published = Arc::new(RecordingInvalidations::default());
            let engine = WorkflowRunEngine::with_run_events(
                SqliteWorkflowRunEngineRepository::new(pool.clone()),
                NoopExecutor,
                SeqGen::default(),
                ClockAt(40),
                published.clone(),
            );
            engine.start(&run_id).unwrap();

            // start→output executes purely in-wave: run start, start-node completion, the
            // output wave, the output completion, and the run finish each commit one transition.
            assert_eq!(
                *published.published.lock().unwrap(),
                vec!["run-1".to_string(); 5]
            );

            let run = SqliteWorkflowRunRepository::new(pool)
                .find_run(&run_id)
                .unwrap()
                .unwrap();
            assert_eq!(run.status, WorkflowRunStatus::Succeeded);
        });
    }

    /// A duplicate or late completion report is an idempotent no-op: the repository rejects the
    /// transition and neither state nor invalidation events change.
    #[test]
    fn duplicate_completion_reports_are_idempotent_no_ops() {
        run_test(async {
            let (temp, pool) = bootstrap();
            let (run_id, node_runs) = started_run(&temp, &pool, TWO_AGENT_GRAPH);
            let left = node_runs
                .iter()
                .find(|node_run| node_run.node_id == "l")
                .unwrap();
            let published = Arc::new(RecordingInvalidations::default());
            let engine = WorkflowRunEngine::with_run_events(
                SqliteWorkflowRunEngineRepository::new(pool.clone()),
                NoopExecutor,
                SeqGen::default(),
                ClockAt(40),
                published.clone(),
            );
            engine
                .complete_node(
                    &run_id,
                    &left.id,
                    Some("done".to_string()),
                    None,
                    None,
                    Vec::new(),
                )
                .unwrap();
            let after_first = published.published.lock().unwrap().len();
            assert!(after_first > 0, "the committed completion publishes");

            // The duplicate report changes nothing: no transition, no event, and the sibling
            // node keeps running.
            engine
                .complete_node(
                    &run_id,
                    &left.id,
                    Some("again".to_string()),
                    None,
                    None,
                    Vec::new(),
                )
                .unwrap();
            assert_eq!(
                published.published.lock().unwrap().len(),
                after_first,
                "a rejected duplicate report must not publish"
            );
            let node_runs = SqliteWorkflowRunRepository::new(pool.clone())
                .list_node_runs(&run_id)
                .unwrap();
            let left = node_runs
                .iter()
                .find(|node_run| node_run.node_id == "l")
                .unwrap();
            let right = node_runs
                .iter()
                .find(|node_run| node_run.node_id == "r")
                .unwrap();
            assert_eq!(
                (left.status, left.output.as_deref()),
                (WorkflowNodeStatus::Succeeded, Some("done"))
            );
            assert_eq!(right.status, WorkflowNodeStatus::Running);
            let run = SqliteWorkflowRunRepository::new(pool)
                .find_run(&run_id)
                .unwrap()
                .unwrap();
            assert_eq!(run.status, WorkflowRunStatus::Running);
        });
    }

    /// Only the Agent runtime creates and binds sessions; every swift control node-run keeps
    /// `session_id` NULL while the control graph finishes synchronously.
    #[test]
    fn control_node_runs_keep_their_session_id_null() {
        run_test(async {
            let (temp, pool) = bootstrap();
            let run_id = seeded_pending_run(&temp, &pool, CONDITION_GRAPH);
            let engine = WorkflowRunEngine::new(
                SqliteWorkflowRunEngineRepository::new(pool.clone()),
                NoopExecutor,
                SeqGen::default(),
                ClockAt(40),
            );
            engine.start(&run_id).unwrap();

            let node_runs = SqliteWorkflowRunRepository::new(pool.clone())
                .list_node_runs(&run_id)
                .unwrap();
            // start, condition, and output all ran and completed without any session binding.
            assert_eq!(node_runs.len(), 3);
            assert!(
                node_runs
                    .iter()
                    .all(|node_run| node_run.session_id.is_none())
            );
            assert!(
                node_runs
                    .iter()
                    .all(|node_run| node_run.status == WorkflowNodeStatus::Succeeded)
            );
            let run = SqliteWorkflowRunRepository::new(pool)
                .find_run(&run_id)
                .unwrap()
                .unwrap();
            assert_eq!(run.status, WorkflowRunStatus::Succeeded);
        });
    }

    /// The backend's invalidation bridge projects each engine publication onto the shared
    /// application event hub, which streams the raw contract event to subscribers.
    #[tokio::test]
    async fn invalidation_bridge_publishes_the_contract_event() {
        let hub = AppEventHub::new();
        let mut stream = hub.subscribe();
        assert_eq!(stream.recv().await.unwrap().unwrap(), AppEvent::Ready);

        let bridge = WorkflowRunInvalidations::new(hub.publisher());
        bridge.publish_run_invalidated(&WorkflowRunId::new("run-9"));

        assert_eq!(
            stream.recv().await.unwrap().unwrap(),
            AppEvent::WorkflowRunInvalidated {
                run_id: "run-9".to_string()
            }
        );
    }
}
