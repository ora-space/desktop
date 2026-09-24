//! Real SQLite fixtures shared by workflow coordination and public lifecycle tests.

use super::interactive::CompletingNodeRuns;
use super::transitions::WorkflowRunTransitions;
use crate::git_cleanup::KeyedResourceLocks;
use ora_application::{
    Clock, ExecutionContext, NodeExecutor, ProjectRepository, SessionRepository,
    SkillMaterializationReceipt, WorkflowGraph, WorkflowGraphNode, WorkflowNodeRunIdGenerator,
    WorkflowRepository, WorkflowRunEngine, WorkflowRunEngineRepository,
    WorkflowRunInvalidationPublisher, WorkflowRunPayload, WorkflowRunRepository,
    WorkflowVariablePool,
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
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

/// Fixture agents disable automatic retry (`agentConfig.retry`) so tests that fail a node see the
/// failure stand at once; retry behaviour has its own fixtures in `retry_tests`.
pub(crate) const AGENT_GRAPH: &str = r#"{"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"agent","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"open_code","modelId":"m"},"prompt":"do"}}}
],"edges":[{"source":"start","target":"agent"}]}"#;

pub(crate) const TWO_AGENT_GRAPH: &str = r#"{"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"l","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"c","modelId":"m"},"prompt":"l"}}},
    {"id":"r","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"c","modelId":"m"},"prompt":"r"}}}
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
        _graph: &WorkflowGraph,
        _context: &ExecutionContext,
        _scope_id: &ora_domain::WorkflowScopeId,
        _variable_pool: &ora_application::WorkflowVariablePool,
    ) {
    }
}

/// Records production dispatch inputs while leaving completion under the test's control.
#[derive(Clone, Default)]
pub(crate) struct RecordingExecutor {
    records: Arc<Mutex<Vec<DispatchRecord>>>,
    /// Node ids in dispatch order, for tests that only care which nodes were started.
    pub dispatches: Arc<Mutex<Vec<String>>>,
}

/// The immutable facts a background executor receives for one dispatch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DispatchRecord {
    pub(crate) node_run_id: String,
    pub(crate) node_id: String,
    pub(crate) payload: Option<String>,
}

impl RecordingExecutor {
    /// Shares a snapshot of all dispatches observed so far.
    pub(crate) fn records(&self) -> Vec<DispatchRecord> {
        self.records.lock().unwrap().clone()
    }
}

impl NodeExecutor for RecordingExecutor {
    fn dispatch(
        &self,
        node_run_id: &WorkflowNodeRunId,
        node: &WorkflowGraphNode,
        _graph: &WorkflowGraph,
        context: &ExecutionContext,
        _scope_id: &ora_domain::WorkflowScopeId,
        _variable_pool: &WorkflowVariablePool,
    ) {
        self.records.lock().unwrap().push(DispatchRecord {
            node_run_id: node_run_id.to_string(),
            node_id: node.id.clone(),
            payload: context.run.payload.clone(),
        });
        self.dispatches.lock().unwrap().push(node.id.clone());
    }
}

#[derive(Default)]
pub(crate) struct SeqGen {
    // Atomic so a fixture engine can be shared across threads; a Mutex around resume would
    // serialize the calls itself and hide whether the run lock is doing that work.
    next: AtomicU64,
}

impl WorkflowNodeRunIdGenerator for SeqGen {
    fn generate_node_run_id(&self) -> WorkflowNodeRunId {
        let current = self.next.fetch_add(1, Ordering::Relaxed);
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

/// Initializes a real git repository in the fixture workspace and returns that root.
pub(crate) fn init_git_workspace(temp: &TempDir) -> PathBuf {
    let root = temp.path().join("fixture-project");
    std::fs::create_dir_all(&root).unwrap();
    run_git(&root, &["init", "--initial-branch=main"]);
    std::fs::write(root.join("README.md"), "seed\n").unwrap();
    run_git(&root, &["add", "README.md"]);
    run_git(&root, &["commit", "-m", "seed"]);
    root
}

fn run_git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(cwd)
        .args([
            "-c",
            "user.name=ora-test",
            "-c",
            "user.email=ora-test@example.com",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
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
    let parsed_graph = WorkflowGraph::parse(graph).unwrap();
    let payload = WorkflowRunPayload::with_variable_pool(
        WorkflowRunLocale::EnUs,
        SkillMaterializationReceipt::default(),
        parsed_graph.start_node().map(|node| node.id.clone()),
        WorkflowVariablePool::from_graph(&parsed_graph),
    );
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
        Some(serde_json::to_string(&payload).unwrap()),
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
    let (run_id, node_runs, _engine) = started_run_with(temp, pool, graph, NoopExecutor);
    (run_id, node_runs)
}

/// Starts a run with a caller-supplied executor so tests can observe dispatches.
pub(crate) fn started_run_with<E: NodeExecutor + 'static>(
    temp: &TempDir,
    pool: &RepositoryPool,
    graph: &str,
    executor: E,
) -> (
    WorkflowRunId,
    Vec<WorkflowNodeRun>,
    WorkflowRunEngine<SqliteWorkflowRunEngineRepository, SeqGen, ClockAt>,
) {
    let run_id = seeded_pending_run(temp, pool, graph);
    let engine = WorkflowRunEngine::new(
        SqliteWorkflowRunEngineRepository::new(pool.clone()),
        executor,
        SeqGen::default(),
        ClockAt(40),
    );
    engine.start(&run_id).unwrap();
    let node_runs = SqliteWorkflowRunRepository::new(pool.clone())
        .list_node_runs(&run_id)
        .unwrap();
    (run_id, node_runs, engine)
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
