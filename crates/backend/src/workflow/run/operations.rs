//! Workflow-run use cases own scheduling gates, manual completion, and session cleanup.

use super::api::WorkflowRunApi;
use super::engine::ConcreteWorkflowRunControl;
use super::interactive::{CompletingNodeRuns, WorkflowSessionTurns};
use crate::agent_runtime::AgentRuntimeManager;
use crate::clock::SystemClock;
use crate::error::BackendError;
use crate::git_cleanup::KeyedResourceLocks;
use crate::repository_work::spawn_repository_work;
use ora_application::{ApplicationError, WorkflowRunEngineRepository};
use ora_contracts::*;
use ora_db::{RepositoryPool, SqliteWorkflowRunEngineRepository};
use ora_logging::ora_warn;
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(test)]
mod tests;

/// Injects the existing engine, runtime, and coordination state without creating another owner.
pub(crate) struct WorkflowRunSetup {
    pub pool: RepositoryPool,
    pub skills_root: PathBuf,
    pub sessions_root: PathBuf,
    pub baselines_root: PathBuf,
    pub agent_runtime: Arc<AgentRuntimeManager>,
    pub engine: Arc<ConcreteWorkflowRunControl>,
    pub run_locks: Arc<KeyedResourceLocks>,
    pub clock: SystemClock,
}

/// Executes complete workflow-run use cases against one shared scheduling/runtime instance.
///
/// Callers do not acquire locks or stop node sessions themselves: terminal transitions commit
/// before best-effort session cleanup, and every scheduling mutation uses the callback's run gate.
pub struct WorkflowRuns {
    pool: RepositoryPool,
    records: WorkflowRunApi,
    sessions_root: PathBuf,
    baselines_root: PathBuf,
    agent_runtime: Arc<AgentRuntimeManager>,
    engine: Arc<ConcreteWorkflowRunControl>,
    run_locks: Arc<KeyedResourceLocks>,
    completing_node_runs: Arc<CompletingNodeRuns>,
}

impl WorkflowRuns {
    /// Captures the instances composed at startup; no lock, worker, or supervisor is restarted.
    pub(crate) fn new(setup: WorkflowRunSetup) -> Self {
        Self {
            records: WorkflowRunApi::new(setup.pool.clone(), setup.skills_root, setup.clock),
            pool: setup.pool,
            sessions_root: setup.sessions_root,
            baselines_root: setup.baselines_root,
            agent_runtime: setup.agent_runtime,
            engine: setup.engine,
            run_locks: setup.run_locks,
            completing_node_runs: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        }
    }

    /// Grants Sessions only the admission/cleanup capability sharing this run's coordination.
    pub(crate) fn session_turns(&self) -> WorkflowSessionTurns {
        WorkflowSessionTurns::new(
            self.pool.clone(),
            self.run_locks.clone(),
            self.completing_node_runs.clone(),
        )
    }

    /// Starts a workflow run against its frozen snapshot graph.
    pub fn start(
        &self,
        request: StartWorkflowRunRequest,
    ) -> Result<StartWorkflowRunResponse, BackendError> {
        let _gate = self.run_locks.acquire_exclusive(request.run_id.clone());
        self.engine.start(request).map_err(BackendError::from)
    }

    /// Cancels a running workflow run and stops its live node sessions.
    ///
    /// The engine commits the `Cancelled` transition first; then every session still bound to the
    /// run's node runs is stopped. Without this second step the agent keeps executing its prompt
    /// and the delete guard treats the lingering `Running` session as an active run.
    pub async fn cancel(
        &self,
        request: CancelWorkflowRunRequest,
    ) -> Result<CancelWorkflowRunResponse, BackendError> {
        let run_id = ora_domain::WorkflowRunId::new(&request.run_id);
        let engine = self.engine.clone();
        let run_locks = self.run_locks.clone();
        let response = spawn_repository_work(move || {
            // Serialize the `Cancelled` transition against every other mutation for the run; the
            // async session cleanup below runs outside the gate.
            let _gate = run_locks.acquire_exclusive(request.run_id.clone());
            engine.cancel(request).map_err(BackendError::from)
        })
        .await?;
        self.stop_workflow_run_sessions(&run_id).await;
        Ok(response)
    }

    /// Stops every agent session started for one run's node runs.
    ///
    /// Best-effort cleanup of an already-cancelled run: a session that cannot be stopped is logged
    /// rather than failing the cancel request, and sessions whose rows were deleted since attach
    /// surface as a warn because `stop_session` can no longer resolve them.
    async fn stop_workflow_run_sessions(&self, run_id: &ora_domain::WorkflowRunId) {
        let pool = self.pool.clone();
        let run_id_for_query = run_id.clone();
        let node_runs = match spawn_repository_work(move || {
            SqliteWorkflowRunEngineRepository::new(pool)
                .list_node_runs(&run_id_for_query)
                .map_err(|source| {
                    BackendError::from(ApplicationError::WorkflowRunRepository { source })
                })
        })
        .await
        {
            Ok(node_runs) => node_runs,
            Err(error) => {
                ora_warn!(run_id = %run_id, error = %error, "cancel: failed to list node runs for session cleanup");
                return;
            }
        };
        for node_run in node_runs {
            let Some(session_id) = node_run.session_id else {
                continue;
            };
            if let Err(error) = self
                .agent_runtime
                .stop_session(StopSessionRequest {
                    session_id: session_id.to_string(),
                })
                .await
            {
                ora_warn!(
                    run_id = %run_id,
                    session_id = %session_id,
                    error = %error,
                    "cancel: failed to stop workflow run session"
                );
            }
        }
    }

    /// Completes one awaiting interactive workflow node as a human request.
    ///
    /// The node is fenced first so no concurrent prompt can start, then its final assistant output
    /// and file diff are read from persisted state, the completion is committed through the engine
    /// under the per-run gate, and finally its session is stopped best-effort. Committing before
    /// stopping means a failed stop can no longer leave a "stopped session but still awaiting node"
    /// gap: once the node is terminal, prompt policy treats the session as read-only.
    pub async fn complete_node(
        &self,
        request: CompleteWorkflowNodeRequest,
    ) -> Result<CompleteWorkflowNodeResponse, BackendError> {
        let run_id = ora_domain::WorkflowRunId::new(&request.run_id);
        let node_id = request.node_id.clone();

        // Fence the node against concurrent prompts and completions before doing any expensive
        // work: once claimed, a prompt is rejected and the worktree stays stable until the commit.
        let claimed_node_run_id = {
            let pool = self.pool.clone();
            let run_locks = self.run_locks.clone();
            let completing = self.completing_node_runs.clone();
            let run_id = run_id.clone();
            let node_id = node_id.clone();
            spawn_repository_work(move || {
                crate::workflow::run::interactive::claim_node_for_completion(
                    &pool,
                    &run_locks,
                    &completing,
                    &run_id,
                    &node_id,
                )
            })
            .await?
        };

        // Prepare the final output and diff outside the gate; on failure release the claim so the
        // node returns to its awaitable state.
        let prepared = {
            let pool = self.pool.clone();
            let sessions_root = self.sessions_root.clone();
            let baselines_root = self.baselines_root.clone();
            let agent_runtime = self.agent_runtime.clone();
            let run_id = run_id.clone();
            let node_id = node_id.clone();
            match spawn_repository_work(move || {
                crate::workflow::run::interactive::prepare_completion(
                    &pool,
                    &sessions_root,
                    &baselines_root,
                    &agent_runtime,
                    &run_id,
                    &node_id,
                )
            })
            .await
            {
                Ok(prepared) => prepared,
                Err(error) => {
                    self.release_completion_claim(&claimed_node_run_id).await;
                    return Err(error);
                }
            }
        };

        // Commit the node completion under the gate and release the claim in the same critical
        // section, so a prompt cannot slip in between the commit and the release. Revalidate first:
        // a cancel that won during prepare must abort this completion rather than report success.
        let pool = self.pool.clone();
        let engine = self.engine.clone();
        let run_locks = self.run_locks.clone();
        let completing = self.completing_node_runs.clone();
        let node_run_id = prepared.node_run_id.clone();
        let output = prepared.output.clone();
        let structured_output = prepared.structured_output.clone();
        let stop_reason = prepared.stop_reason.clone();
        let file_changes = prepared.file_changes.clone();
        let response = spawn_repository_work(move || {
            let _gate = run_locks.acquire_exclusive(run_id.as_ref());
            let result = crate::workflow::run::interactive::revalidate_completion(
                &pool,
                &run_id,
                &node_run_id,
            )
            .and_then(|()| {
                engine
                    .complete_node(
                        &run_id,
                        &node_run_id,
                        output,
                        structured_output,
                        stop_reason,
                        file_changes,
                    )
                    .map_err(BackendError::from)
            });
            completing
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&node_run_id);
            result
        })
        .await?;

        // The node is terminal now, so its worktree baseline is no longer needed for a diff.
        let baseline_path = self
            .baselines_root
            .join(format!("{}.json", prepared.node_run_id.as_ref()));
        let _ = spawn_repository_work(move || {
            std::fs::remove_file(baseline_path).ok();
            Ok(())
        })
        .await;

        // Stop the session best-effort after the commit: the node is terminal now, so a failure
        // here only leaves a lingering session that prompt policy already treats as read-only.
        if let Some(session_id) = prepared.session_id.as_ref()
            && let Err(error) = self
                .agent_runtime
                .stop_session(StopSessionRequest {
                    session_id: session_id.to_string(),
                })
                .await
        {
            ora_warn!(session_id = %session_id, error = %error, "complete: failed to stop completed node session");
        }

        Ok(response)
    }

    /// Releases a completion claim after a prepare failure, returning the node to its awaitable
    /// state. Best-effort: a poisoned or contended completing set must not mask the real error.
    async fn release_completion_claim(&self, node_run_id: &ora_domain::WorkflowNodeRunId) {
        let completing = self.completing_node_runs.clone();
        let node_run_id = node_run_id.clone();
        let _ = spawn_repository_work(move || {
            completing
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&node_run_id);
            Ok(())
        })
        .await;
    }

    /// Restarts a finished workflow run.
    pub fn restart(
        &self,
        request: RestartWorkflowRunRequest,
    ) -> Result<RestartWorkflowRunResponse, BackendError> {
        let _gate = self.run_locks.acquire_exclusive(request.run_id.clone());
        self.engine.restart(request).map_err(BackendError::from)
    }

    /// Sets the kickoff input of a pending workflow run.
    pub fn update_input(
        &self,
        request: UpdateWorkflowRunInputRequest,
    ) -> Result<UpdateWorkflowRunInputResponse, BackendError> {
        let _gate = self.run_locks.acquire_exclusive(request.run_id.clone());
        self.engine
            .update_input(request)
            .map_err(BackendError::from)
    }

    /// Creates one workflow run through the shared application composition.
    pub fn create(
        &self,
        request: CreateWorkflowRunRequest,
    ) -> Result<CreateWorkflowRunResponse, BackendError> {
        self.records.create(request).map_err(BackendError::from)
    }
    /// Gets one workflow run through the shared application composition.
    pub fn get(
        &self,
        request: GetWorkflowRunRequest,
    ) -> Result<GetWorkflowRunResponse, BackendError> {
        self.records.get(request).map_err(BackendError::from)
    }
    /// Lists workflow runs for one project through the shared application composition.
    pub fn list(
        &self,
        request: ListWorkflowRunsRequest,
    ) -> Result<ListWorkflowRunsResponse, BackendError> {
        self.records.list(request).map_err(BackendError::from)
    }
    /// Lists workflow runs for one workflow through the shared application composition.
    pub fn list_by_workflow(
        &self,
        request: ListWorkflowRunsByWorkflowRequest,
    ) -> Result<ListWorkflowRunsByWorkflowResponse, BackendError> {
        self.records
            .list_by_workflow(request)
            .map_err(BackendError::from)
    }
    /// Lists the node-run history of one run through the shared application composition.
    pub fn list_node_runs(
        &self,
        request: ListWorkflowNodeRunsRequest,
    ) -> Result<ListWorkflowNodeRunsResponse, BackendError> {
        self.records
            .list_node_runs(request)
            .map_err(BackendError::from)
    }
    /// Deletes one workflow run through the shared application composition.
    pub fn delete(
        &self,
        request: DeleteWorkflowRunRequest,
    ) -> Result<DeleteWorkflowRunResponse, BackendError> {
        self.records.delete(request).map_err(BackendError::from)
    }

    /// Renames one workflow run through its Workspace-owned display field.
    pub fn rename(
        &self,
        request: RenameWorkflowRunRequest,
    ) -> Result<RenameWorkflowRunResponse, BackendError> {
        self.records.rename(request).map_err(BackendError::from)
    }
}
