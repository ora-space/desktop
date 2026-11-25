//! Execution coordination for run-local debugging (ADR D5): from a confirmed preview to the new
//! attempts actually dispatching. The shared store (#1428) owns receipts, revisions and history;
//! this coordinator owns the dispatch fence, quiescing the affected scope, the atomic apply, the
//! invalidation notice and the re-dispatch. Independent branches keep running throughout.

use super::engine::ConcreteWorkflowRunEngine;
use super::transitions::WorkflowRunTransitions;
use crate::agent_runtime::AgentRuntimeManager;
use crate::clock::SystemClock;
use crate::error::BackendError;
use crate::git_cleanup::KeyedResourceLocks;
use ora_application::{ApplicationError, Clock, RepositoryError, WorkflowRunEngineRepository};
use ora_contracts::{
    CommitWorkflowDebugRequest, PreviewWorkflowDebugRequest, StopSessionRequest,
    WorkflowDebugOperation, WorkflowDebugPreview,
};
use ora_db::workflow_debug::{DebugClaim, WorkflowDebugStore};
use ora_db::{RepositoryPool, SqliteWorkflowRunEngineRepository};
use ora_domain::{WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRunId};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

mod strategy;

/// Maps a debug-store failure onto the backend error surface like the other run use cases.
fn store_error(error: ora_db::DatabaseError) -> BackendError {
    ApplicationError::WorkflowRunRepository {
        source: RepositoryError::new(error),
    }
    .into()
}

/// Stops one bound session and proves it unloaded. The agent runtime implements it; coordination
/// tests inject a double so the fence/apply/re-dispatch path runs without a live session host.
pub(crate) trait DebugSessionStopper: Send + Sync {
    /// Stops the session, resolving only once the runtime has unloaded it (or reporting why not).
    fn stop<'a>(
        &'a self,
        session_id: String,
    ) -> Pin<Box<dyn Future<Output = Result<(), BackendError>> + Send + 'a>>;
}

impl DebugSessionStopper for AgentRuntimeManager {
    fn stop<'a>(
        &'a self,
        session_id: String,
    ) -> Pin<Box<dyn Future<Output = Result<(), BackendError>> + Send + 'a>> {
        Box::pin(async move {
            self.stop_session(StopSessionRequest { session_id }).await?;
            Ok(())
        })
    }
}

/// Drives one debug confirmation through fence → quiesce → apply → notify → re-dispatch.
pub(crate) struct WorkflowDebugCoordinator {
    pool: RepositoryPool,
    run_locks: Arc<KeyedResourceLocks>,
    stopper: Arc<dyn DebugSessionStopper>,
    engine: Arc<ConcreteWorkflowRunEngine>,
    transitions: Arc<WorkflowRunTransitions>,
    clock: SystemClock,
}

impl WorkflowDebugCoordinator {
    /// Composes the coordinator from the same runtime, gate and engine the run use cases share.
    pub(crate) fn new(
        pool: RepositoryPool,
        run_locks: Arc<KeyedResourceLocks>,
        stopper: Arc<dyn DebugSessionStopper>,
        engine: Arc<ConcreteWorkflowRunEngine>,
        transitions: Arc<WorkflowRunTransitions>,
        clock: SystemClock,
    ) -> Self {
        Self {
            pool,
            run_locks,
            stopper,
            engine,
            transitions,
            clock,
        }
    }

    /// Plans a read-only preview through the top-level strategy; no execution side effects.
    pub(crate) fn preview(
        &self,
        request: PreviewWorkflowDebugRequest,
    ) -> Result<WorkflowDebugPreview, BackendError> {
        WorkflowDebugStore::new(self.pool.clone())
            .preview(&request, strategy::plan)
            .map_err(store_error)
    }

    /// Confirms a preview and carries it to the new dispatch, or returns the durable operation.
    ///
    /// Only the client whose claim is `Claimed` drives stopping; a reconnect or duplicate reads the
    /// same operation back. The affected scope is fenced by the durable `stopping` operation, its
    /// in-flight attempts are cancelled and stopped outside the run gate, and the revision applies
    /// in one commit under the gate before the invalidation notice and the scheduling wave.
    pub(crate) async fn commit(
        &self,
        request: CommitWorkflowDebugRequest,
    ) -> Result<WorkflowDebugOperation, BackendError> {
        let store = WorkflowDebugStore::new(self.pool.clone());
        let run_id = WorkflowRunId::new(&request.run_id);
        let entry = match store.claim(&request).map_err(store_error)? {
            DebugClaim::Claimed(entry) => entry,
            DebugClaim::Existing(entry) => return Ok(entry.operation),
        };

        let stop_ids: Vec<WorkflowNodeRunId> = entry
            .preview
            .impact
            .stop
            .iter()
            .map(WorkflowNodeRunId::new)
            .collect();

        // Cancel the affected attempts under the gate so their late callbacks and retries become
        // no-ops through the existing status guards, without failing the run or its siblings.
        let now = self.clock.now_timestamp_millis();
        {
            let _gate = self.run_locks.acquire_exclusive(request.run_id.clone());
            let repository = SqliteWorkflowRunEngineRepository::new(self.pool.clone());
            for id in &stop_ids {
                let _ = repository.transition_node_run_status(
                    id,
                    WorkflowNodeStatus::Running,
                    WorkflowNodeStatus::Cancelled,
                    now,
                );
            }
        }

        // Stop the bound sessions outside the gate: a driver callback acquires the gate on its own,
        // so waiting here must never hold it. A stop that cannot be proven records stop_failed and
        // keeps the fence; no revision and no new dispatch follow.
        let repository = SqliteWorkflowRunEngineRepository::new(self.pool.clone());
        for id in &stop_ids {
            let session_id = repository
                .find_node_run_by_id(id)
                .map_err(|source| {
                    BackendError::from(ApplicationError::WorkflowRunRepository { source })
                })?
                .and_then(|node| node.session_id);
            if let Some(session_id) = session_id
                && let Err(error) = self.stopper.stop(session_id.to_string()).await
            {
                let reason = format!("failed to stop attempt {id}: {error}");
                let _ = store.record_stop_failure(&request.run_id, &request.request_id, &reason);
                let DebugClaim::Existing(failed) = store.claim(&request).map_err(store_error)?
                else {
                    return Err(store_error(
                        ora_db::DatabaseError::IncompleteWorkflowRunContext,
                    ));
                };
                return Ok(failed.operation);
            }
        }

        // Apply the revision and the projection in one commit under the gate, then notify and
        // re-dispatch. The fence lifts with the applied operation, so the wave reads the new scope.
        let applied = {
            let _gate = self.run_locks.acquire_exclusive(request.run_id.clone());
            store
                .apply_after_stop(&request.run_id, &request.request_id, |tx, preview| {
                    strategy::settle(tx, preview)
                })
                .map_err(store_error)?
        };
        self.transitions.publish_run_invalidated(&run_id);
        {
            let _gate = self.run_locks.acquire_exclusive(request.run_id.clone());
            self.engine
                .resume(&run_id)
                .map_err(ApplicationError::from_workflow_engine_error)?;
        }
        Ok(applied)
    }
}
