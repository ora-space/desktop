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
use ora_application::{ApplicationError, Clock, NodeType, WorkflowRunEngineRepository};
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

pub(super) mod strategy;

/// Maps a debug-store failure onto the backend error surface like the other run use cases.
fn store_error(error: ora_db::DatabaseError) -> BackendError {
    BackendError::from_debug_store(error)
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

    /// Refuses a normal scheduling entry point while a debug confirmation holds the fence (ADR D5.1):
    /// failure recovery, restart and snapshot switch must not write, re-root or dispatch under an
    /// active stopping or unreleased failed stop.
    pub(crate) fn ensure_no_fence(&self, run_id: &str) -> Result<(), BackendError> {
        let store = WorkflowDebugStore::new(self.pool.clone());
        store
            .ensure_debug_operation_inactive(run_id)
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
        let stop_ids = {
            let _gate = self.run_locks.acquire_exclusive(request.run_id.clone());
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
                let repository = SqliteWorkflowRunEngineRepository::new(self.pool.clone());
                for id in &stop_ids {
                    // Invalidate the parked manual action before a stop can release the run gate.
                    for from in [WorkflowNodeStatus::Pending, WorkflowNodeStatus::Running] {
                        repository
                            .transition_node_run_status(
                                id,
                                from,
                                WorkflowNodeStatus::Cancelled,
                                now,
                            )
                            .map_err(|source| ApplicationError::WorkflowRunRepository { source })?;
                    }
                }
            }

            // Stop the bound sessions outside the gate: a driver callback acquires the gate on its own,
            // so waiting here must never hold it. A stop that cannot be proven records stop_failed and
            // keeps the fence; no revision and no new dispatch follow.
            stop_ids
        };
        let repository = SqliteWorkflowRunEngineRepository::new(self.pool.clone());
        for (index, id) in stop_ids.iter().enumerate() {
            let row = repository.find_node_run_by_id(id).map_err(|source| {
                BackendError::from(ApplicationError::WorkflowRunRepository { source })
            })?;
            let result = match row {
                Some(ora_domain::WorkflowNodeRun {
                    session_id: Some(session_id),
                    ..
                }) => self
                    .stopper
                    .stop(session_id.to_string())
                    .await
                    .map_err(|e| e.to_string()),
                // Engine-owned containers have a separate stop entry for each external member.
                Some(node)
                    if matches!(
                        node.node_type.parse(),
                        Ok(NodeType::Iteration | NodeType::Loop)
                    ) =>
                {
                    Ok(())
                }
                Some(node)
                    if node.started_at.is_none()
                        && node
                            .payload
                            .as_deref()
                            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                            .is_some_and(|v| v[ora_application::RETRY_WAIT_KEY].is_object()) =>
                {
                    Ok(())
                }
                _ => Err("cannot prove isolation before session binding".to_owned()),
            };
            if let Err(error) = result {
                let reason = format!("failed to stop attempt {id}: {error}");
                store
                    .record_stop_failure(
                        &request.run_id,
                        &request.request_id,
                        &reason,
                        &stop_ids[index..],
                    )
                    .map_err(store_error)?;
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
        let _gate = self.run_locks.acquire_exclusive(request.run_id.clone());
        let now = self.clock.now_timestamp_millis();
        let applied = store
            .apply_after_stop(&request.run_id, &request.request_id, now, |tx, preview| {
                strategy::settle(tx, &request.run_id, now, preview)
            })
            .map_err(store_error)?;
        if matches!(
            applied.phase,
            ora_contracts::WorkflowDebugPhase::Applied { .. }
        ) {
            self.transitions.publish_run_invalidated(&run_id);
            drop(_gate);
            let _dispatch_gate = self.run_locks.acquire_exclusive(request.run_id.clone());
            self.engine
                .resume(&run_id)
                .map_err(ApplicationError::from_workflow_engine_error)?;
        } else if matches!(&applied.phase,ora_contracts::WorkflowDebugPhase::StopFailed {reason,..} if reason=="run_cancelled")
        {
            // All the requested stops just acknowledged. Releasing the old fence grants no
            // dispatch and does not replace the cancellation receipt.
            store
                .release_failed_stop(&request.run_id, &request.request_id)
                .map_err(store_error)?;
        }
        Ok(applied)
    }
}
