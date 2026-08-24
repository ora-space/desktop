//! Coordinates human follow-up turns against interactive workflow node sessions.
//!
//! An interactive node parks at `Pending` (awaiting input) after its first turn. When the user
//! sends a follow-up message through the ordinary `prompt_session` path, this module validates the
//! node against the per-run gate, flips it to `Running` while the agent answers, and flips it back
//! to `Pending` when the turn ends or the stream is dropped. Terminal nodes are read-only: a
//! session bound to a `Succeeded`/`Failed`/`Cancelled` node no longer accepts prompts through this
//! path, which keeps completed workflow nodes from mutating the worktree with no node-run
//! provenance.

use crate::clock::SystemClock;
use crate::error::BackendError;
use crate::git_cleanup::KeyedResourceLocks;
use ora_application::{
    AdvanceWorkflowRunResult, ApplicationError, Clock, RepositoryError, WorkflowRunEngineRepository,
};
use ora_db::{RepositoryPool, SqliteWorkflowRunEngineRepository};
use ora_domain::{SessionId, WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRunStatus};
use std::sync::Arc;

use super::CompletingNodeRuns;

/// Validates and flips an awaiting interactive node to `Running` before a human turn.
///
/// Returns `Some(node_run_id)` when the session is bound to an awaiting interactive node that is
/// now running; `None` when the session is bound to no workflow node (an ordinary session prompt).
/// Rejects with `WorkflowNodeNotAwaitingInput` when the node is terminal, already running, being
/// completed, or its run is no longer executing, so a finished workflow node can never accept
/// another prompt through this path.
pub(crate) async fn begin_human_turn(
    pool: &RepositoryPool,
    run_locks: &Arc<KeyedResourceLocks>,
    completing_node_runs: &Arc<CompletingNodeRuns>,
    session_id: &str,
) -> Result<Option<WorkflowNodeRunId>, BackendError> {
    let pool = pool.clone();
    let run_locks = run_locks.clone();
    let completing_node_runs = completing_node_runs.clone();
    let session_id = SessionId::new(session_id);
    tokio::task::spawn_blocking(move || {
        let repository = SqliteWorkflowRunEngineRepository::new(pool);
        let node_run = repository
            .find_node_run_by_session_id(&session_id)
            .map_err(repository_error)?;
        let Some(node_run) = node_run else {
            // No workflow node is bound to this session: an ordinary session prompt.
            return Ok(None);
        };
        // Serialize against completion/cancel/scheduling for this run before deciding whether the
        // prompt may start, so a concurrent completion cannot be raced past.
        let _gate = run_locks.acquire_exclusive(node_run.run_id.as_ref());
        // Re-read the node under the gate: its status may have changed since the first read.
        let node_run = repository
            .find_node_run_by_session_id(&session_id)
            .map_err(repository_error)?;
        let Some(node_run) = node_run else {
            return Ok(None);
        };
        // The owning run must still be executing; a terminal run rejects a follow-up turn.
        let context = repository
            .find_execution_context(&node_run.run_id)
            .map_err(repository_error)?;
        let run_running = context
            .as_ref()
            .is_some_and(|context| context.run.status == WorkflowRunStatus::Running);
        if !run_running {
            return Err(node_not_awaiting(&node_run.node_id));
        }
        // A terminal node is read-only; a non-Pending node is not awaiting input.
        if node_run.status != WorkflowNodeStatus::Pending {
            return Err(node_not_awaiting(&node_run.node_id));
        }
        if completing_node_runs
            .lock()
            .map_err(|_poisoned| node_not_awaiting(&node_run.node_id))?
            .contains(&node_run.id)
        {
            return Err(node_not_awaiting(&node_run.node_id));
        }
        // The guarded transition both grants the turn and races against any concurrent mutation;
        // a rejected transition means the node is no longer awaiting and the prompt must not start.
        match repository
            .transition_node_run_status(
                &node_run.id,
                WorkflowNodeStatus::Pending,
                WorkflowNodeStatus::Running,
                SystemClock.now_timestamp_millis(),
            )
            .map_err(repository_error)?
        {
            AdvanceWorkflowRunResult::Advanced => Ok(Some(node_run.id)),
            AdvanceWorkflowRunResult::NotRunning | AdvanceWorkflowRunResult::NotFound => {
                Err(node_not_awaiting(&node_run.node_id))
            }
        }
    })
    .await
    .map_err(|source| BackendError::internal("repository operation did not complete", source))?
}

/// Flips an interactive node back to `Pending` when a turn ends or its stream is dropped.
///
/// This is deliberately exempt from the per-run gate: it is a guarded `Running → Pending` cleanup
/// transition that computes no ready set, dispatches nothing, and becomes a no-op when the node has
/// already reached a terminal state. Routing the fire-and-forget drop hook through the gate would
/// add a run-id lookup and a blocking lock for no scheduling benefit.
pub(crate) async fn end_human_turn(
    pool: &RepositoryPool,
    node_run_id: &WorkflowNodeRunId,
) -> Result<(), BackendError> {
    let pool = pool.clone();
    let node_run_id = node_run_id.clone();
    tokio::task::spawn_blocking(move || {
        let repository = SqliteWorkflowRunEngineRepository::new(pool);
        repository
            .transition_node_run_status(
                &node_run_id,
                WorkflowNodeStatus::Running,
                WorkflowNodeStatus::Pending,
                SystemClock.now_timestamp_millis(),
            )
            .map_err(repository_error)?;
        Ok(())
    })
    .await
    .map_err(|source| BackendError::internal("repository operation did not complete", source))?
}

/// Renders the public rejection for a workflow node that cannot accept a prompt turn.
fn node_not_awaiting(node_id: &str) -> BackendError {
    BackendError::from(ApplicationError::WorkflowNodeNotAwaitingInput {
        node_id: node_id.to_string(),
    })
}

/// Maps a workflow engine repository failure onto the public backend error.
fn repository_error(source: RepositoryError) -> BackendError {
    BackendError::from(ApplicationError::WorkflowRunRepository { source })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ora_application::{
        Clock, ExecutionContext, NodeExecutor, ProjectRepository, WorkflowGraphNode,
        WorkflowNodeRunIdGenerator, WorkflowRepository, WorkflowRunEngine, WorkflowRunRepository,
    };
    use ora_db::{
        DatabaseBootstrapper, DatabaseLocation, SqliteProjectRepository, SqliteWorkflowRepository,
        SqliteWorkflowRunRepository, default_migration_catalog,
    };
    use ora_domain::{
        AuditFields, Namespace, Project, ProjectId, Task, TaskId, Workflow, WorkflowId,
        WorkflowNodeRun, WorkflowRun, WorkflowRunId, WorkflowSnapshot, WorkflowSnapshotId,
        Worktree, WorktreeActivity, WorktreeBaseline, WorktreeId, WorktreeProvisioningLeaseId,
    };
    use pretty_assertions::assert_eq;
    use std::cell::Cell;
    use std::collections::HashSet;
    use tempfile::TempDir;

    const AGENT_GRAPH: &str = r#"{"nodes":[
        {"id":"start","data":{"kind":"start"}},
        {"id":"agent","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"open_code","modelId":"m"},"prompt":"do"}}}
    ],"edges":[{"source":"start","target":"agent"}]}"#;

    const TWO_AGENT_GRAPH: &str = r#"{"nodes":[
        {"id":"start","data":{"kind":"start"}},
        {"id":"l","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"l"}}},
        {"id":"r","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"r"}}}
    ],"edges":[{"source":"start","target":"l"},{"source":"start","target":"r"}]}"#;

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

    #[derive(Default)]
    struct SeqGen {
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
    struct ClockAt(i64);

    impl Clock for ClockAt {
        fn now_timestamp_millis(&self) -> i64 {
            self.0
        }
    }

    fn bootstrap() -> (TempDir, RepositoryPool) {
        let temp = TempDir::new().unwrap();
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(&temp.path().join("repository.sqlite3")),
                &default_migration_catalog().expect("create migration catalog"),
            )
            .expect("bootstrap repository pool");
        (temp, pool)
    }

    /// Seeds a project, workflow, snapshot, and pending run, then starts it so the agent nodes are
    /// `Running`. Returns the run id and the started run's node runs.
    fn started_run(pool: &RepositoryPool, graph: &str) -> (WorkflowRunId, Vec<WorkflowNodeRun>) {
        let project = SqliteProjectRepository::new(pool.clone());
        project
            .create_project(Project::new(
                ProjectId::new("project-1"),
                "Fixture project",
                "/tmp/fixture-project",
                AuditFields::new(1, 1, false),
            ))
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

        let run_id = WorkflowRunId::new("run-1");
        let task_id = TaskId::new("task-1");
        let worktree_id = WorktreeId::new("worktree-1");
        let run = WorkflowRun::new(
            run_id.clone(),
            workflow.id,
            snapshot.id,
            WorkflowRunStatus::Pending,
            Some("{\"current_nodes\":[]}".to_string()),
            Some("kickoff".to_string()),
            None,
            None,
            None,
            None,
            None,
            AuditFields::new(30, 30, false),
        );
        let task = Task::workflow_run(
            task_id.clone(),
            ProjectId::new("project-1"),
            "Workflow run".to_string(),
            run_id.clone(),
            worktree_id.clone(),
            AuditFields::new(30, 30, false),
        );
        let worktree = Worktree::new(
            worktree_id,
            task_id,
            Some("ora/task-1".to_string()),
            None,
            WorktreeBaseline::recorded("base-commit").unwrap(),
            WorktreeActivity::Active,
            AuditFields::new(30, 30, false),
        );
        SqliteWorkflowRunRepository::new(pool.clone())
            .create_run(
                run,
                task,
                worktree,
                &WorktreeProvisioningLeaseId::new("lease-absent"),
            )
            .unwrap();

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

    fn locks() -> (Arc<KeyedResourceLocks>, Arc<CompletingNodeRuns>) {
        (
            KeyedResourceLocks::new(),
            Arc::new(std::sync::Mutex::new(HashSet::new())),
        )
    }

    /// Binds a session to an agent node and parks it at `Pending`, returning the node id.
    fn bind_and_park(
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

    /// A session not bound to any workflow node is an ordinary session prompt.
    #[tokio::test]
    async fn session_without_bound_node_is_an_ordinary_prompt() {
        let (_temp, pool) = bootstrap();
        let (run_locks, completing) = locks();
        let result = begin_human_turn(&pool, &run_locks, &completing, "unbound-session")
            .await
            .unwrap();
        assert_eq!(result, None);
    }

    /// A session bound to a terminal node rejects the prompt instead of proceeding as ordinary.
    #[tokio::test]
    async fn terminal_node_rejects_prompt() {
        let (_temp, pool) = bootstrap();
        let (_run_id, node_runs) = started_run(&pool, AGENT_GRAPH);
        let agent = node_runs.iter().find(|n| n.node_id == "agent").unwrap();
        let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
        let session_id = SessionId::new("session-1");
        repository
            .bind_node_run_session(&agent.id, &session_id, 50)
            .unwrap();
        repository
            .transition_node_run_status(
                &agent.id,
                WorkflowNodeStatus::Running,
                WorkflowNodeStatus::Succeeded,
                50,
            )
            .unwrap();

        let (run_locks, completing) = locks();
        assert!(
            begin_human_turn(&pool, &run_locks, &completing, "session-1")
                .await
                .is_err(),
            "a terminal node must reject the prompt"
        );
    }

    /// A node being manually completed rejects a concurrent prompt.
    #[tokio::test]
    async fn completing_node_rejects_prompt() {
        let (_temp, pool) = bootstrap();
        let (_run_id, node_runs) = started_run(&pool, AGENT_GRAPH);
        let agent = node_runs.iter().find(|n| n.node_id == "agent").unwrap();
        let (session_id, node_run_id) = bind_and_park(&pool, agent);

        let (run_locks, completing) = locks();
        completing.lock().unwrap().insert(node_run_id);

        assert!(
            begin_human_turn(&pool, &run_locks, &completing, session_id.as_ref())
                .await
                .is_err(),
            "a completing node must reject the prompt"
        );
    }

    /// An awaiting node flips to `Running` and admits the prompt.
    #[tokio::test]
    async fn awaiting_node_flips_to_running() {
        let (_temp, pool) = bootstrap();
        let (_run_id, node_runs) = started_run(&pool, AGENT_GRAPH);
        let agent = node_runs.iter().find(|n| n.node_id == "agent").unwrap();
        let (session_id, _node_run_id) = bind_and_park(&pool, agent);

        let (run_locks, completing) = locks();
        let result = begin_human_turn(&pool, &run_locks, &completing, session_id.as_ref())
            .await
            .unwrap();
        assert!(result.is_some());

        let node_runs = SqliteWorkflowRunRepository::new(pool)
            .list_node_runs(&_run_id)
            .unwrap();
        let agent = node_runs.iter().find(|n| n.node_id == "agent").unwrap();
        assert_eq!(agent.status, WorkflowNodeStatus::Running);
    }

    /// A `Pending` node in a failed run rejects the prompt (the run is no longer executing).
    #[tokio::test]
    async fn non_running_run_rejects_prompt() {
        let (_temp, pool) = bootstrap();
        let (run_id, node_runs) = started_run(&pool, TWO_AGENT_GRAPH);
        let left = node_runs.iter().find(|n| n.node_id == "l").unwrap();
        let right = node_runs.iter().find(|n| n.node_id == "r").unwrap();
        let (session_id, _node_run_id) = bind_and_park(&pool, left);

        // Failing the sibling node fails the run but leaves the parked node `Pending`.
        let engine = WorkflowRunEngine::new(
            SqliteWorkflowRunEngineRepository::new(pool.clone()),
            NoopExecutor,
            SeqGen::default(),
            ClockAt(40),
        );
        engine
            .fail_node(&right.id, "boom".to_string(), None)
            .unwrap();

        let (run_locks, completing) = locks();
        assert!(
            begin_human_turn(&pool, &run_locks, &completing, session_id.as_ref())
                .await
                .is_err(),
            "a node in a non-running run must reject the prompt"
        );
        assert_ne!(run_id.as_ref(), "");
    }

    /// A second completion claim against the same awaiting node is rejected, so two concurrent
    /// completes cannot both prepare against one node.
    #[test]
    fn second_completion_claim_is_rejected() {
        let (_temp, pool) = bootstrap();
        let (run_id, node_runs) = started_run(&pool, AGENT_GRAPH);
        let agent = node_runs.iter().find(|n| n.node_id == "agent").unwrap();
        let (_session_id, _node_run_id) = bind_and_park(&pool, agent);

        let (run_locks, completing) = locks();

        assert!(
            super::super::completion::claim_node_for_completion(
                &pool,
                &run_locks,
                &completing,
                &run_id,
                "agent",
            )
            .is_ok()
        );

        assert!(
            super::super::completion::claim_node_for_completion(
                &pool,
                &run_locks,
                &completing,
                &run_id,
                "agent",
            )
            .is_err(),
            "a second claim against the same awaiting node must be rejected"
        );
    }
}
