//! Real SQLite fixtures shared by workflow coordination and public lifecycle tests.

use super::interactive::CompletingNodeRuns;
use super::transitions::WorkflowRunTransitions;
use crate::git_cleanup::KeyedResourceLocks;
use ora_application::{
    Clock, ExecutionContext, NodeExecutor, ProjectRepository, SessionRepository, WorkflowGraphNode,
    WorkflowNodeRunIdGenerator, WorkflowRepository, WorkflowRunEngine, WorkflowRunEngineRepository,
    WorkflowRunInvalidationPublisher, WorkflowRunPayload, WorkflowRunRepository,
};
use ora_contracts::WorkflowRunLocale;
use ora_db::{
    DatabaseBootstrapper, DatabaseLocation, SqliteProjectRepository, SqliteSessionRepository,
    SqliteWorkflowRepository, SqliteWorkflowRunRepository, SqliteWorkspaceRepository,
    default_migration_catalog,
};
use ora_db::{RepositoryPool, SqliteWorkflowRunEngineRepository};
use ora_domain::{
    AgentRef, AuditFields, Namespace, Project, ProjectId, Session, SessionId, SessionStatus,
    Workflow, WorkflowId, WorkflowNodeRun, WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRun,
    WorkflowRunId, WorkflowRunStatus, WorkflowSnapshot, WorkflowSnapshotId, WorkspaceLocation,
};
use std::cell::Cell;
use std::collections::HashSet;
use std::sync::Arc;
use tempfile::TempDir;

pub(crate) const AGENT_GRAPH: &str = r#"{"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"agent","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"open_code","modelId":"m"},"prompt":"do"}}}
],"edges":[{"source":"start","target":"agent"}]}"#;

pub(crate) const TWO_AGENT_GRAPH: &str = r#"{"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"l","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"l"}}},
    {"id":"r","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"r"}}}
],"edges":[{"source":"start","target":"l"},{"source":"start","target":"r"}]}"#;

/// A purely swift chain: the whole run completes inside scheduling waves, with no dispatch.
pub(crate) const CONTROL_GRAPH: &str = r#"{"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"out","data":{"kind":"output"}}
],"edges":[{"source":"start","target":"out"}]}"#;

/// A swift chain through a Condition: the empty case list always selects the else branch.
pub(crate) const CONDITION_GRAPH: &str = r#"{"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"c","data":{"kind":"condition"}},
    {"id":"out","data":{"kind":"output"}}
],"edges":[
    {"source":"start","target":"c"},
    {"source":"c","sourceHandle":"else","target":"out"}
]}"#;

pub(crate) struct NoopExecutor;

impl NodeExecutor for NoopExecutor {
    fn dispatch(
        &self,
        _node_run_id: &WorkflowNodeRunId,
        _node: &WorkflowGraphNode,
        _context: &ExecutionContext,
    ) {
    }
}

#[derive(Default)]
pub(crate) struct SeqGen {
    next: Cell<u64>,
}

impl WorkflowNodeRunIdGenerator for SeqGen {
    fn generate_node_run_id(&self) -> WorkflowNodeRunId {
        let current = self.next.get();
        self.next.set(current + 1);
        WorkflowNodeRunId::new(format!("node-{current}"))
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ClockAt(pub(crate) i64);

impl Clock for ClockAt {
    fn now_timestamp_millis(&self) -> i64 {
        self.0
    }
}

/// Opens a migrated SQLite fixture independent of a live backend runtime.
pub(crate) fn bootstrap() -> (TempDir, RepositoryPool) {
    let temp = TempDir::new().unwrap();
    let pool = DatabaseBootstrapper::new(crate::test_clock::TestClock)
        .bootstrap_repository_pool(
            &DatabaseLocation::path(&temp.path().join("repository.sqlite3")),
            &default_migration_catalog().expect("create migration catalog"),
        )
        .expect("bootstrap repository pool");
    (temp, pool)
}

/// Seeds a project, workflow, snapshot, and a `Pending` run without starting it, so callers
/// can start the run through their own engine composition.
pub(crate) fn seeded_pending_run(
    temp: &TempDir,
    pool: &RepositoryPool,
    graph: &str,
) -> WorkflowRunId {
    let workspace_path = temp.path().join("fixture-project");
    std::fs::create_dir_all(&workspace_path).unwrap();
    let project = SqliteProjectRepository::with_clock(pool.clone(), crate::test_clock::TestClock);
    project
        .create_project(
            Project::new(
                ProjectId::new("project-1"),
                "Fixture project",
                AuditFields::new(1, 1, false),
            ),
            WorkspaceLocation::local_filesystem(workspace_path.to_string_lossy()),
        )
        .unwrap();
    let workflow_repo = SqliteWorkflowRepository::new(pool.clone());
    let workflow = Workflow::new(
        WorkflowId::new("workflow-1"),
        Namespace::local(),
        "Workflow".to_string(),
        /*published_snapshot_id*/ None,
        AuditFields::new(10, 10, false),
    )
    .unwrap();
    let draft = WorkflowSnapshot::new(
        WorkflowSnapshotId::new("draft"),
        workflow.id.clone(),
        "draft",
        graph,
        10,
        Some(10),
        false,
    );
    workflow_repo
        .create_workflow(workflow.clone(), draft.clone())
        .unwrap();
    let snapshot = WorkflowSnapshot::new(
        WorkflowSnapshotId::new("snapshot-1"),
        workflow.id.clone(),
        "v1",
        graph,
        20,
        None,
        false,
    );
    workflow_repo
        .publish_snapshot(
            &workflow.id,
            snapshot.id.clone(),
            snapshot.version.clone(),
            snapshot.created_at,
        )
        .unwrap();

    let workspace = SqliteWorkspaceRepository::new(pool.clone())
        .find_main_workspace(&ProjectId::new("project-1"))
        .unwrap()
        .unwrap();
    SqliteSessionRepository::new(pool.clone())
        .create_session(Session::new(
            SessionId::new("session-1"),
            workspace.id.clone(),
            AgentRef::parse("ora-space.opencode").unwrap(),
            "provider-session-1",
            SessionStatus::Stopped,
            ora_domain::SessionMcpSelection::Automatic,
            AuditFields::new(25, 25, false),
        ))
        .unwrap();
    let run_id = WorkflowRunId::new("run-1");
    // Real runs are created through the deployment handler, which always freezes a typed
    // payload; without one, private routing state such as Condition decisions would never
    // persist, so the fixture seeds the graph-derived pool exactly like deployment does.
    let parsed_graph = ora_application::WorkflowGraph::parse(graph).expect("fixture graph parses");
    let start_node_id = parsed_graph.start_node().map(|node| node.id.clone());
    let payload = serde_json::to_string(&WorkflowRunPayload::with_variable_pool(
        WorkflowRunLocale::EnUs,
        Default::default(),
        start_node_id,
        ora_application::WorkflowVariablePool::from_graph(&parsed_graph),
    ))
    .unwrap();
    let run = WorkflowRun::new(
        run_id.clone(),
        workspace.id,
        workflow.id,
        snapshot.id,
        "Workflow run",
        WorkflowRunStatus::Pending,
        Some("{\"current_nodes\":[]}".to_string()),
        Some("kickoff".to_string()),
        None,
        None,
        Some(payload),
        None,
        None,
        AuditFields::new(30, 30, false),
    );
    SqliteWorkflowRunRepository::new(pool.clone())
        .create_run(run)
        .unwrap();
    run_id
}

/// Seeds a project, workflow, snapshot, and pending run, then starts it so the agent nodes are
/// `Running`. Returns the run id and the started run's node runs.
pub(crate) fn started_run(
    temp: &TempDir,
    pool: &RepositoryPool,
    graph: &str,
) -> (WorkflowRunId, Vec<WorkflowNodeRun>) {
    let run_id = seeded_pending_run(temp, pool, graph);
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
    (run_id, node_runs)
}

/// Creates isolated coordination state for tests of the internal turn-policy interface.
pub(crate) fn locks() -> (Arc<KeyedResourceLocks>, Arc<CompletingNodeRuns>) {
    (
        KeyedResourceLocks::new(),
        Arc::new(std::sync::Mutex::new(HashSet::new())),
    )
}

/// Binds a session to an agent node and parks it at `Pending`, returning the node id.
pub(crate) fn bind_and_park(
    pool: &RepositoryPool,
    node_run: &WorkflowNodeRun,
) -> (SessionId, WorkflowNodeRunId) {
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let session_id = SessionId::new("session-1");
    repository
        .bind_node_run_session(&node_run.id, &session_id, 50)
        .unwrap();
    repository
        .transition_node_run_status(
            &node_run.id,
            WorkflowNodeStatus::Running,
            WorkflowNodeStatus::Pending,
            50,
        )
        .unwrap();
    (session_id, node_run.id.clone())
}

/// Captures the run ids each invalidation identified, for tests of the D7 event channel.
pub(crate) struct RecordingInvalidations {
    pub(crate) published: std::sync::Mutex<Vec<String>>,
}

impl Default for RecordingInvalidations {
    fn default() -> Self {
        Self {
            published: std::sync::Mutex::new(Vec::new()),
        }
    }
}

impl WorkflowRunInvalidationPublisher for RecordingInvalidations {
    fn publish_run_invalidated(&self, run_id: &WorkflowRunId) {
        self.published.lock().unwrap().push(run_id.to_string());
    }
}

/// Builds a transition sink over the fixture pool whose invalidations are recorded, so tests
/// assert the D7 publish discipline of engine-external commits.
pub(crate) fn recording_transitions(
    pool: &RepositoryPool,
) -> (Arc<WorkflowRunTransitions>, Arc<RecordingInvalidations>) {
    let recording = Arc::new(RecordingInvalidations::default());
    let transitions = Arc::new(WorkflowRunTransitions::new(pool.clone(), recording.clone()));
    (transitions, recording)
}

/// Keeps bootstrap and every emitting operation under the same scoped TRACE subscriber.
pub(crate) fn run_test(test: impl std::future::Future<Output = ()>) {
    ora_logging::with_trace_logging(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(test);
    });
}
