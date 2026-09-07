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
    use crate::workflow::run::test_fixture::*;
    use ora_application::{WorkflowRunEngine, WorkflowRunRepository};
    use ora_db::SqliteWorkflowRunRepository;
    use pretty_assertions::assert_eq;
    /// A session not bound to any workflow node is an ordinary session prompt.
    #[test]
    fn session_without_bound_node_is_an_ordinary_prompt() {
        run_test(async {
            let (_temp, pool) = bootstrap();
            let (run_locks, completing) = locks();
            let result = begin_human_turn(&pool, &run_locks, &completing, "unbound-session")
                .await
                .unwrap();
            assert_eq!(result, None);
        });
    }

    /// A session bound to a terminal node rejects the prompt instead of proceeding as ordinary.
    #[test]
    fn terminal_node_rejects_prompt() {
        run_test(async {
            let (temp, pool) = bootstrap();
            let (_run_id, node_runs) = started_run(&temp, &pool, AGENT_GRAPH);
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
        });
    }

    /// A node being manually completed rejects a concurrent prompt.
    #[test]
    fn completing_node_rejects_prompt() {
        run_test(async {
            let (temp, pool) = bootstrap();
            let (_run_id, node_runs) = started_run(&temp, &pool, AGENT_GRAPH);
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
        });
    }

    /// An awaiting node flips to `Running` and admits the prompt.
    #[test]
    fn awaiting_node_flips_to_running() {
        run_test(async {
            let (temp, pool) = bootstrap();
            let (_run_id, node_runs) = started_run(&temp, &pool, AGENT_GRAPH);
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
        });
    }

    /// A `Pending` node in a failed run rejects the prompt (the run is no longer executing).
    #[test]
    fn non_running_run_rejects_prompt() {
        run_test(async {
            let (temp, pool) = bootstrap();
            let (run_id, node_runs) = started_run(&temp, &pool, TWO_AGENT_GRAPH);
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
        });
    }

    /// A second completion claim against the same awaiting node is rejected, so two concurrent
    /// completes cannot both prepare against one node.
    #[test]
    fn second_completion_claim_is_rejected() {
        run_test(async {
            let (temp, pool) = bootstrap();
            let (run_id, node_runs) = started_run(&temp, &pool, AGENT_GRAPH);
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
        });
    }
}
