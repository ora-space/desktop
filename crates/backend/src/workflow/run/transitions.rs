//! Node-run status transitions committed outside the scheduling engine.
//!
//! The interactive chain — an agent node parking at awaiting input after its first turn, a
//! human follow-up turn beginning, and that turn ending — commits guarded node-run transitions
//! directly instead of entering `WorkflowRunEngine`. This sink keeps those commits on the same
//! discipline as the engine's own transitions (ADR "node runtime orchestration" D7): the
//! guarded transition commits first, and exactly one run invalidation is published when it
//! commits. A rejected guard is an idempotent no-op that commits nothing and publishes
//! nothing.

use crate::clock::SystemClock;
use ora_application::{
    AdvanceWorkflowRunResult, Clock, RepositoryError, WorkflowRunEngineRepository,
    WorkflowRunInvalidationPublisher,
};
use ora_db::{RepositoryPool, SqliteWorkflowRunEngineRepository};
use ora_domain::{WorkflowNodeRunId, WorkflowNodeStatus};
use std::sync::Arc;

/// Commits the engine-external node-run transitions and publishes their run invalidations.
///
/// The sink shares the engine's invalidation mechanism (the same
/// [`WorkflowRunInvalidationPublisher`] bridge onto the application event hub), so observers
/// cannot tell an engine-committed transition from an interactive one — both re-query the same
/// persisted state.
pub(crate) struct WorkflowRunTransitions {
    pool: RepositoryPool,
    events: Arc<dyn WorkflowRunInvalidationPublisher>,
}

impl WorkflowRunTransitions {
    /// Creates the sink over the run persistence and the shared invalidation mechanism.
    pub(crate) fn new(
        pool: RepositoryPool,
        events: Arc<dyn WorkflowRunInvalidationPublisher>,
    ) -> Self {
        Self { pool, events }
    }

    /// Commits one guarded node-run status transition, publishing the owning run's
    /// invalidation only when the repository accepted it.
    ///
    /// The run id is resolved from the committed row: callers such as the turn-cleanup path
    /// hold only the node-run id. A row that vanishes between the commit and the read (a
    /// concurrent restart) publishes nothing — a lost event only leaves a stale view that the
    /// next transition or refresh clears.
    pub(crate) fn transition_node_run_status(
        &self,
        node_run_id: &WorkflowNodeRunId,
        from: WorkflowNodeStatus,
        to: WorkflowNodeStatus,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError> {
        let repository = SqliteWorkflowRunEngineRepository::new(self.pool.clone());
        let result = repository.transition_node_run_status(
            node_run_id,
            from,
            to,
            SystemClock.now_timestamp_millis(),
        )?;
        if !matches!(result, AdvanceWorkflowRunResult::Advanced) {
            return Ok(result);
        }
        if let Some(node_run) = repository.find_node_run_by_id(node_run_id)? {
            self.events.publish_run_invalidated(&node_run.run_id);
        }
        Ok(result)
    }

    /// Publishes one run invalidation after a commit that happened outside this sink.
    ///
    /// Resume snapshot-switch commits through the engine repository rather than this sink; the
    /// caller publishes afterwards so observers re-query the switched snapshot (ADR D7).
    pub(crate) fn publish_run_invalidated(&self, run_id: &ora_domain::WorkflowRunId) {
        self.events.publish_run_invalidated(run_id);
    }
}

#[cfg(test)]
mod tests {
    use super::WorkflowRunTransitions;
    use crate::workflow::run::test_fixture::{
        AGENT_GRAPH, RecordingInvalidations, bootstrap, run_test, started_run,
    };
    use ora_application::WorkflowRunRepository;
    use ora_db::SqliteWorkflowRunRepository;
    use ora_domain::WorkflowNodeStatus;
    use pretty_assertions::assert_eq;
    use std::sync::Arc;

    /// A committed transition publishes exactly one invalidation carrying the owning run id:
    /// the same discipline the engine applies to its own scheduling-wave transitions.
    #[test]
    fn committed_transitions_publish_one_invalidation() {
        run_test(async {
            let (temp, pool) = bootstrap();
            let (run_id, node_runs) = started_run(&temp, &pool, AGENT_GRAPH);
            let agent = node_runs.iter().find(|n| n.node_id == "agent").unwrap();
            let recording = Arc::new(RecordingInvalidations::default());
            let transitions = WorkflowRunTransitions::new(pool.clone(), recording.clone());

            let result = transitions.transition_node_run_status(
                &agent.id,
                WorkflowNodeStatus::Running,
                WorkflowNodeStatus::Pending,
            );

            assert_eq!(
                result.unwrap(),
                ora_application::AdvanceWorkflowRunResult::Advanced
            );
            assert_eq!(
                *recording.published.lock().unwrap(),
                vec![run_id.to_string()],
                "a committed transition publishes exactly one invalidation"
            );
            let parked = SqliteWorkflowRunRepository::new(pool)
                .list_node_runs(&run_id)
                .unwrap()
                .into_iter()
                .find(|n| n.node_id == "agent")
                .unwrap();
            assert_eq!(parked.status, WorkflowNodeStatus::Pending);
        });
    }

    /// A rejected guard (the node is no longer in the expected status) is an idempotent
    /// no-op: nothing commits, so nothing publishes.
    #[test]
    fn rejected_transitions_publish_nothing() {
        run_test(async {
            let (temp, pool) = bootstrap();
            let (_run_id, node_runs) = started_run(&temp, &pool, AGENT_GRAPH);
            let agent = node_runs.iter().find(|n| n.node_id == "agent").unwrap();
            let recording = Arc::new(RecordingInvalidations::default());
            let transitions = WorkflowRunTransitions::new(pool.clone(), recording.clone());

            // The node is Running, so a Pending -> Running guard must reject.
            let result = transitions.transition_node_run_status(
                &agent.id,
                WorkflowNodeStatus::Pending,
                WorkflowNodeStatus::Running,
            );

            assert_eq!(
                result.unwrap(),
                ora_application::AdvanceWorkflowRunResult::NotRunning
            );
            assert!(
                recording.published.lock().unwrap().is_empty(),
                "an idempotently rejected transition must not publish"
            );
        });
    }
}
