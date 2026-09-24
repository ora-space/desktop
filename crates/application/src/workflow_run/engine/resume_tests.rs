use super::{
    AdvanceWorkflowRunResult, BindWorkflowNodeSessionResult, CancelWorkflowRunResult,
    ExecutionContext, FileChange, NodeExecutor, NodeRunToStart, RestartWorkflowRunResult,
    ResumeWorkflowRunResult, StartWorkflowRunResult, UpdateWorkflowRunInputResult,
    WorkflowGraphNode, WorkflowNodeRunIdGenerator, WorkflowRunEngine, WorkflowRunEngineRepository,
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
        {"id":"out","data":{"kind":"output"}}
    ],
    "edges": [
        {"source":"start","target":"a"},
        {"source":"start","target":"b"},
        {"source":"a","target":"c"},
        {"source":"c","target":"out"},
        {"source":"b","target":"out"}
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

/// Records the node ids the engine asks the repository to clear.
struct RecordingRepository {
    context: ExecutionContext,
    node_runs: Vec<WorkflowNodeRun>,
    cleared: Arc<Mutex<Option<Vec<String>>>>,
}

/// These tests fail nodes only with kinds that never retry automatically.
impl crate::workflow_run::engine::WorkflowRetryRepository for RecordingRepository {
    fn schedule_node_retry(
        &self,
        _failed_node_run_id: &WorkflowNodeRunId,
        _failure: &crate::workflow_run::engine::NodeFailure,
        _retry: &crate::workflow_run::engine::NodeRetryToSchedule,
        _now: i64,
    ) -> Result<crate::workflow_run::engine::ScheduleNodeRetryResult, RepositoryError> {
        unreachable!("no automatic retry is scheduled in these tests")
    }

    fn begin_node_retry(
        &self,
        _node_run_id: &WorkflowNodeRunId,
        _now: i64,
    ) -> Result<crate::workflow_run::engine::BeginNodeRetryResult, RepositoryError> {
        unreachable!("no automatic retry is scheduled in these tests")
    }
}

impl WorkflowRunEngineRepository for RecordingRepository {
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
        Ok(Some(self.context.clone()))
    }

    fn list_node_runs(
        &self,
        _run_id: &WorkflowRunId,
    ) -> Result<Vec<WorkflowNodeRun>, RepositoryError> {
        Ok(self.node_runs.clone())
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
        _node_run_id: &WorkflowNodeRunId,
        _session_id: &SessionId,
        _now: i64,
    ) -> Result<BindWorkflowNodeSessionResult, RepositoryError> {
        Ok(BindWorkflowNodeSessionResult::NotFound)
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
        _node_runs: &[NodeRunToStart],
        _now: i64,
    ) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn complete_node(
        &self,
        _node_run_id: &WorkflowNodeRunId,
        _output: Option<String>,
        _structured_output: Option<serde_json::Value>,
        _stop_reason: Option<String>,
        _file_changes: Vec<FileChange>,
        _now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError> {
        Ok(AdvanceWorkflowRunResult::NotFound)
    }

    fn fail_node(
        &self,
        _node_run_id: &WorkflowNodeRunId,
        _failure: super::NodeFailure,
        _propagation: super::FailurePropagation,
        _now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError> {
        Ok(AdvanceWorkflowRunResult::NotFound)
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
        node_ids_to_clear: &[String],
        _now: i64,
    ) -> Result<ResumeWorkflowRunResult, RepositoryError> {
        *self.cleared.lock().expect("cleared lock") = Some(node_ids_to_clear.to_vec());
        // The persisted context stays Failed, so the follow-up schedule wave is a no-op.
        Ok(ResumeWorkflowRunResult::Resumed)
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

fn execution_context() -> ExecutionContext {
    execution_context_with(GRAPH)
}

fn execution_context_with(graph_json: &str) -> ExecutionContext {
    ExecutionContext {
        root_scope_id: ora_domain::WorkflowScopeId::new("root:test"),
        run: WorkflowRun::new(
            WorkflowRunId::new("run-1"),
            WorkspaceId::new("workspace-1"),
            WorkflowId::new("workflow-1"),
            WorkflowSnapshotId::new("snapshot-1"),
            "Review",
            WorkflowRunStatus::Failed,
            Some(r#"{"current_nodes":["a"]}"#.to_string()),
            Some("task".to_string()),
            None,
            Some("a failed".to_string()),
            None,
            Some(10),
            Some(20),
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
        graph_json: graph_json.to_string(),
    }
}

fn node_run(id: &str, node_id: &str, status: WorkflowNodeStatus) -> WorkflowNodeRun {
    WorkflowNodeRun::new(
        WorkflowNodeRunId::new(id),
        WorkflowRunId::new("run-1"),
        ora_domain::WorkflowScopeId::new("root:test"),
        node_id,
        "agent",
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

fn engine(
    node_runs: Vec<WorkflowNodeRun>,
) -> (
    WorkflowRunEngine<RecordingRepository, SeqGen, ClockAt>,
    Arc<Mutex<Option<Vec<String>>>>,
) {
    engine_with(node_runs, GRAPH)
}

fn engine_with(
    node_runs: Vec<WorkflowNodeRun>,
    graph_json: &str,
) -> (
    WorkflowRunEngine<RecordingRepository, SeqGen, ClockAt>,
    Arc<Mutex<Option<Vec<String>>>>,
) {
    let cleared = Arc::new(Mutex::new(None));
    let repository = RecordingRepository {
        context: execution_context_with(graph_json),
        node_runs,
        cleared: cleared.clone(),
    };
    (
        WorkflowRunEngine::new(repository, NoopExecutor, SeqGen, ClockAt(40)),
        cleared,
    )
}

/// A run with only succeeded node runs has nothing to resume from.
#[test]
fn resume_from_failure_is_not_resumable_without_failed_nodes() {
    let (engine, cleared) = engine(vec![
        node_run("nr-start", "start", WorkflowNodeStatus::Succeeded),
        node_run("nr-a", "a", WorkflowNodeStatus::Succeeded),
        node_run("nr-b", "b", WorkflowNodeStatus::Succeeded),
    ]);
    assert_eq!(
        engine
            .resume_from_failure(&WorkflowRunId::new("run-1"))
            .unwrap(),
        ResumeWorkflowRunResult::NotResumable
    );
    assert_eq!(*cleared.lock().expect("cleared lock"), None);
}

/// The engine clears the failed node and every transitive successor, not the sibling branch.
#[test]
fn resume_from_failure_clears_failed_nodes_and_successors_not_siblings() {
    let (engine, cleared) = engine(vec![
        node_run("nr-start", "start", WorkflowNodeStatus::Succeeded),
        node_run("nr-a", "a", WorkflowNodeStatus::Failed),
        node_run("nr-b", "b", WorkflowNodeStatus::Succeeded),
    ]);
    assert_eq!(
        engine
            .resume_from_failure(&WorkflowRunId::new("run-1"))
            .unwrap(),
        ResumeWorkflowRunResult::Resumed
    );
    assert_eq!(
        *cleared.lock().expect("cleared lock"),
        Some(vec!["a".to_string(), "c".to_string(), "out".to_string()])
    );
}

const ITERATION_GRAPH: &str = r#"{
    "nodes": [
        {"id":"start","data":{"kind":"start","inputVariables":[{"name":"prs","valueType":"array[object]"}]}},
        {"id":"other","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"other"}}},
        {"id":"iter","data":{"kind":"iteration","iterationConfig":{
            "iteratorSelector":["start","prs"],
            "collectSelector":["fix","output"],
            "errorStrategy":"fail",
            "maxIterations":10
        }}},
        {"id":"fix","parentId":"iter","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"fix"}}},
        {"id":"out","data":{"kind":"output"}}
    ],
    "edges": [
        {"source":"start","target":"other"},
        {"source":"start","target":"iter"},
        {"source":"iter","target":"fix"},
        {"source":"iter","target":"out"}
    ]
}"#;

/// A failed region member restarts the owning composite, every round of every member, and
/// outer descendants; a succeeded outer sibling is left alone.
#[test]
fn resume_from_failure_clears_the_composite_unit_and_leaves_outer_siblings() {
    let (engine, cleared) = engine_with(
        vec![
            node_run("nr-start", "start", WorkflowNodeStatus::Succeeded),
            node_run("nr-other", "other", WorkflowNodeStatus::Succeeded),
            node_run("nr-iter", "iter", WorkflowNodeStatus::Failed),
            node_run("nr-fix-0", "fix", WorkflowNodeStatus::Succeeded).in_iteration(Some(0)),
            node_run("nr-fix-1", "fix", WorkflowNodeStatus::Failed).in_iteration(Some(1)),
        ],
        ITERATION_GRAPH,
    );
    assert_eq!(
        engine
            .resume_from_failure(&WorkflowRunId::new("run-1"))
            .unwrap(),
        ResumeWorkflowRunResult::Resumed
    );
    assert_eq!(
        *cleared.lock().expect("cleared lock"),
        Some(vec![
            "fix".to_string(),
            "iter".to_string(),
            "out".to_string()
        ])
    );
}
