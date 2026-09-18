use super::{
    AdvanceWorkflowRunResult, BindWorkflowNodeSessionResult, CancelWorkflowRunResult,
    ExecutionContext, FileChange, NodeExecutor, NodeFailure, NodeFailureKind, NodeRunToStart,
    RestartWorkflowRunResult, ResumeWorkflowRunResult, StartWorkflowRunResult,
    UpdateWorkflowRunInputResult, WorkflowGraphNode, WorkflowNodeRunIdGenerator, WorkflowRunEngine,
    WorkflowRunEngineRepository,
};
use crate::RepositoryError;
use crate::project::Clock;
use crate::workflow_run::engine::graph::WorkflowGraph;
use crate::workflow_run::engine::variable_pool::WorkflowVariablePool;
use ora_domain::{
    AuditFields, SessionId, WorkflowId, WorkflowNodeRun, WorkflowNodeRunId, WorkflowNodeStatus,
    WorkflowRun, WorkflowRunId, WorkflowRunStatus, WorkflowSnapshotId, Workspace, WorkspaceId,
    WorkspaceKind, WorkspaceLifecycle, WorkspaceLocation,
};
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

const GRAPH: &str = r#"{
    "nodes": [
        {"id":"start","data":{"kind":"start"}},
        {"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"a"}}},
        {"id":"b","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"b"}}},
        {"id":"c","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"c"}}},
        {"id":"output","data":{"kind":"output"}}
    ],
    "edges": [
        {"source":"start","target":"a"},
        {"source":"start","target":"b"},
        {"source":"a","target":"c"},
        {"source":"b","target":"c"},
        {"source":"c","target":"output"}
    ]
}"#;

struct NoopExecutor;

impl NodeExecutor for NoopExecutor {
    fn dispatch(
        &self,
        _node_run_id: &WorkflowNodeRunId,
        _node: &WorkflowGraphNode,
        _graph: &WorkflowGraph,
        _context: &ExecutionContext,
        _scope_id: &ora_domain::WorkflowScopeId,
        _variable_pool: &WorkflowVariablePool,
    ) {
    }
}

struct SeqGen;

impl WorkflowNodeRunIdGenerator for SeqGen {
    fn generate_node_run_id(&self) -> WorkflowNodeRunId {
        WorkflowNodeRunId::new("unused")
    }
}

struct ClockAt(i64);

impl Clock for ClockAt {
    fn now_timestamp_millis(&self) -> i64 {
        self.0
    }
}

struct EngineState {
    context: ExecutionContext,
    node_runs: Vec<WorkflowNodeRun>,
    started_ready: Vec<String>,
    finish_run_calls: usize,
}

/// In-memory engine repository that mirrors D2: bind succeeds while the node is Running even if
/// the run is already Failed.
#[derive(Clone)]
struct InMemoryRepository {
    state: Arc<Mutex<EngineState>>,
}

impl InMemoryRepository {
    fn lock(&self) -> std::sync::MutexGuard<'_, EngineState> {
        self.state.lock().expect("engine state lock")
    }
}

impl WorkflowRunEngineRepository for InMemoryRepository {
    fn find_active_loop_round(
        &self,
        _parent_loop_node_run_id: &WorkflowNodeRunId,
    ) -> Result<Option<ora_domain::WorkflowExecutionScope>, RepositoryError> {
        Ok(None)
    }

    fn list_node_runs_in_scope(
        &self,
        _scope_id: &ora_domain::WorkflowScopeId,
    ) -> Result<Vec<WorkflowNodeRun>, RepositoryError> {
        // Tests model a single root scope, so the scope listing is the run listing.
        self.list_node_runs(&WorkflowRunId::new("run-1"))
    }

    fn start_loop_round(
        &self,
        _run_id: &WorkflowRunId,
        _round: &crate::workflow_run::engine::ports::LoopRoundToStart,
        _now: i64,
    ) -> Result<(), RepositoryError> {
        unreachable!("no Loop node in these graphs")
    }

    fn start_scope_ready_nodes(
        &self,
        _scope_id: &ora_domain::WorkflowScopeId,
        _node_runs: &[NodeRunToStart],
        _now: i64,
    ) -> Result<(), RepositoryError> {
        unreachable!("no Loop node in these graphs")
    }

    fn advance_loop_round(
        &self,
        _scope_id: &ora_domain::WorkflowScopeId,
        _advance: &crate::workflow_run::engine::ports::LoopRoundAdvance,
        _now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError> {
        unreachable!("no Loop node in these graphs")
    }

    fn find_execution_context(
        &self,
        _run_id: &WorkflowRunId,
    ) -> Result<Option<ExecutionContext>, RepositoryError> {
        Ok(Some(self.lock().context.clone()))
    }

    fn list_node_runs(
        &self,
        _run_id: &WorkflowRunId,
    ) -> Result<Vec<WorkflowNodeRun>, RepositoryError> {
        Ok(self.lock().node_runs.clone())
    }

    fn find_last_failed_attempt(
        &self,
        _run_id: &WorkflowRunId,
        _node_id: &str,
        _iteration: Option<u32>,
    ) -> Result<Option<WorkflowNodeRun>, RepositoryError> {
        Ok(None)
    }

    fn bind_node_run_session(
        &self,
        node_run_id: &WorkflowNodeRunId,
        session_id: &SessionId,
        _now: i64,
    ) -> Result<BindWorkflowNodeSessionResult, RepositoryError> {
        let mut state = self.lock();
        let Some(node) = state
            .node_runs
            .iter_mut()
            .find(|node| node.id == *node_run_id)
        else {
            return Ok(BindWorkflowNodeSessionResult::NotFound);
        };
        if node.status != WorkflowNodeStatus::Running {
            return Ok(BindWorkflowNodeSessionResult::NotRunning);
        }
        node.session_id = Some(session_id.clone());
        Ok(BindWorkflowNodeSessionResult::Bound)
    }

    fn find_node_run_by_session_id(
        &self,
        _session_id: &SessionId,
    ) -> Result<Option<WorkflowNodeRun>, RepositoryError> {
        Ok(None)
    }

    fn find_node_run_by_id(
        &self,
        node_run_id: &WorkflowNodeRunId,
    ) -> Result<Option<WorkflowNodeRun>, RepositoryError> {
        Ok(self
            .lock()
            .node_runs
            .iter()
            .find(|node| node.id == *node_run_id)
            .cloned())
    }

    fn transition_node_run_status(
        &self,
        _node_run_id: &WorkflowNodeRunId,
        _from: WorkflowNodeStatus,
        _to: WorkflowNodeStatus,
        _now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError> {
        Ok(AdvanceWorkflowRunResult::NotFound)
    }

    fn start_run(
        &self,
        _run_id: &WorkflowRunId,
        _start_node_run: &NodeRunToStart,
        _now: i64,
    ) -> Result<StartWorkflowRunResult, RepositoryError> {
        Ok(StartWorkflowRunResult::Current)
    }

    fn start_ready_nodes(
        &self,
        _run_id: &WorkflowRunId,
        node_runs: &[NodeRunToStart],
        _now: i64,
    ) -> Result<(), RepositoryError> {
        self.lock()
            .started_ready
            .extend(node_runs.iter().map(|node_run| node_run.node_id.clone()));
        Ok(())
    }

    fn complete_node(
        &self,
        node_run_id: &WorkflowNodeRunId,
        output: Option<String>,
        _structured_output: Option<serde_json::Value>,
        _stop_reason: Option<String>,
        _file_changes: Vec<FileChange>,
        _now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError> {
        let mut state = self.lock();
        let Some(node) = state
            .node_runs
            .iter_mut()
            .find(|node| node.id == *node_run_id)
        else {
            return Ok(AdvanceWorkflowRunResult::NotFound);
        };
        if !matches!(
            node.status,
            WorkflowNodeStatus::Running | WorkflowNodeStatus::Pending
        ) {
            return Ok(AdvanceWorkflowRunResult::NotRunning);
        }
        node.status = WorkflowNodeStatus::Succeeded;
        node.output = output;
        Ok(AdvanceWorkflowRunResult::Advanced)
    }

    fn fail_node(
        &self,
        node_run_id: &WorkflowNodeRunId,
        failure: NodeFailure,
        propagation: super::FailurePropagation,
        _now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError> {
        let mut state = self.lock();
        let Some(node) = state
            .node_runs
            .iter_mut()
            .find(|node| node.id == *node_run_id)
        else {
            return Ok(AdvanceWorkflowRunResult::NotFound);
        };
        if node.status != WorkflowNodeStatus::Running {
            return Ok(AdvanceWorkflowRunResult::NotRunning);
        }
        node.status = WorkflowNodeStatus::Failed;
        node.error = Some(failure.message);
        if propagation == super::FailurePropagation::Run {
            state.context.run.status = WorkflowRunStatus::Failed;
        }
        Ok(AdvanceWorkflowRunResult::Advanced)
    }

    fn start_iteration_round(
        &self,
        _run_id: &WorkflowRunId,
        _owner_node_id: &str,
        _round: u32,
        _item: &serde_json::Value,
        _node_runs: &[NodeRunToStart],
        _now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError> {
        Ok(AdvanceWorkflowRunResult::NotFound)
    }

    fn settle_iteration_round(
        &self,
        _run_id: &WorkflowRunId,
        _owner_node_id: &str,
        _round: u32,
        _entry: super::RoundOutcome,
        _continuation: super::IterationRoundContinuation,
        _now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError> {
        Ok(AdvanceWorkflowRunResult::NotFound)
    }

    fn complete_iteration_node(
        &self,
        _node_run_id: &WorkflowNodeRunId,
        _owner_node_id: &str,
        _exposed: &[(String, serde_json::Value)],
        _output: Option<String>,
        _now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError> {
        Ok(AdvanceWorkflowRunResult::NotFound)
    }

    fn record_node_checkpoint(
        &self,
        _node_run_id: &WorkflowNodeRunId,
        _snapshot_id: &str,
        _checkpoint: Option<&str>,
        _checkpoint_error: Option<&str>,
        _now: i64,
    ) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn record_node_injected_failure(
        &self,
        _node_run_id: &WorkflowNodeRunId,
        _text: &str,
    ) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn record_node_ai_diagnosis(
        &self,
        _node_run_id: &WorkflowNodeRunId,
        _diagnosis_json: &str,
        _now: i64,
    ) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn finish_run(
        &self,
        _run_id: &WorkflowRunId,
        _output: Option<String>,
        _now: i64,
    ) -> Result<(), RepositoryError> {
        self.lock().finish_run_calls += 1;
        Ok(())
    }

    fn cancel_run(
        &self,
        _run_id: &WorkflowRunId,
        _now: i64,
    ) -> Result<CancelWorkflowRunResult, RepositoryError> {
        Ok(CancelWorkflowRunResult::NotFound)
    }

    fn restart_run(
        &self,
        _run_id: &WorkflowRunId,
        _now: i64,
    ) -> Result<RestartWorkflowRunResult, RepositoryError> {
        Ok(RestartWorkflowRunResult::NotFound)
    }

    fn resume_from_failure(
        &self,
        _run_id: &WorkflowRunId,
        _node_ids_to_clear: &[String],
        _now: i64,
    ) -> Result<ResumeWorkflowRunResult, RepositoryError> {
        Ok(ResumeWorkflowRunResult::NotResumable)
    }

    fn switch_run_snapshot(
        &self,
        _run_id: &WorkflowRunId,
        _snapshot_id: &WorkflowSnapshotId,
        _payload_json: &str,
        _now: i64,
    ) -> Result<bool, RepositoryError> {
        Ok(true)
    }

    fn update_run_input(
        &self,
        _run_id: &WorkflowRunId,
        _input: Option<String>,
        _variables: BTreeMap<String, serde_json::Value>,
        _now: i64,
    ) -> Result<UpdateWorkflowRunInputResult, RepositoryError> {
        Ok(UpdateWorkflowRunInputResult::NotFound)
    }

    fn list_recoverable_runs(&self) -> Result<Vec<WorkflowRunId>, RepositoryError> {
        Ok(Vec::new())
    }

    fn fail_orphaned_node_runs(
        &self,
        _run_ids: &[WorkflowRunId],
        _now: i64,
    ) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn fail_interrupted_node_runs(
        &self,
        _run_id: &WorkflowRunId,
        _node_run_ids: &[WorkflowNodeRunId],
        _now: i64,
    ) -> Result<(), RepositoryError> {
        Ok(())
    }
}

fn execution_context(status: WorkflowRunStatus) -> ExecutionContext {
    ExecutionContext {
        root_scope_id: ora_domain::WorkflowScopeId::new("root:test"),
        run: WorkflowRun::new(
            WorkflowRunId::new("run-1"),
            WorkspaceId::new("workspace-1"),
            WorkflowId::new("workflow-1"),
            WorkflowSnapshotId::new("snapshot-1"),
            "Review",
            status,
            Some(r#"{"current_nodes":["a","b"]}"#.to_string()),
            Some("task".to_string()),
            None,
            None,
            None,
            Some(10),
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
        graph_json: GRAPH.to_string(),
    }
}

fn node_run(
    id: &str,
    node_id: &str,
    node_type: &str,
    status: WorkflowNodeStatus,
) -> WorkflowNodeRun {
    WorkflowNodeRun::new(
        WorkflowNodeRunId::new(id),
        WorkflowRunId::new("run-1"),
        ora_domain::WorkflowScopeId::new("root:test"),
        node_id,
        node_type,
        None,
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

/// A late success after a sibling failure stays persisted; the Failed run does not dispatch C.
#[test]
fn failed_run_binds_in_flight_sibling_and_does_not_dispatch_join() {
    let repository = InMemoryRepository {
        state: Arc::new(Mutex::new(EngineState {
            context: execution_context(WorkflowRunStatus::Running),
            node_runs: vec![
                node_run("nr-start", "start", "start", WorkflowNodeStatus::Succeeded),
                node_run("nr-a", "a", "agent", WorkflowNodeStatus::Running),
                node_run("nr-b", "b", "agent", WorkflowNodeStatus::Running),
            ],
            started_ready: Vec::new(),
            finish_run_calls: 0,
        })),
    };
    let engine = WorkflowRunEngine::new(repository.clone(), NoopExecutor, SeqGen, ClockAt(40));
    let run_id = WorkflowRunId::new("run-1");
    let a_id = WorkflowNodeRunId::new("nr-a");
    let b_id = WorkflowNodeRunId::new("nr-b");

    engine
        .fail_node(
            &run_id,
            &b_id,
            NodeFailure::new(NodeFailureKind::PromptTemplate, "b prompt failed"),
        )
        .unwrap();
    assert_eq!(
        repository
            .bind_node_run_session(&a_id, &SessionId::new("session-a"), 50)
            .unwrap(),
        BindWorkflowNodeSessionResult::Bound
    );
    engine
        .complete_node(
            &run_id,
            &a_id,
            Some("a-output".to_string()),
            /*structured_output*/ None,
            Some("end_turn".to_string()),
            Vec::new(),
        )
        .unwrap();

    let state = repository.lock();
    let node_a = state
        .node_runs
        .iter()
        .find(|node| node.node_id == "a")
        .unwrap();
    assert_eq!(node_a.status, WorkflowNodeStatus::Succeeded);
    assert_eq!(node_a.output.as_deref(), Some("a-output"));
    assert_eq!(node_a.session_id, Some(SessionId::new("session-a")));
    assert!(state.node_runs.iter().all(|node| node.node_id != "c"));
    assert_eq!(state.context.run.status, WorkflowRunStatus::Failed);
    assert_eq!(state.started_ready, Vec::<String>::new());
    assert_eq!(state.finish_run_calls, 0);
}
