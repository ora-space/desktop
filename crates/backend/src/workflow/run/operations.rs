//! Workflow-run use cases own scheduling gates, manual completion, and session cleanup.

use super::api::WorkflowRunApi;
use super::engine::ConcreteWorkflowRunControl;
use super::interactive::{CompletingNodeRuns, CompletionApproval, WorkflowSessionTurns};
#[cfg(feature = "test-support")]
use super::owner_stream_drop::OwnerStreamDropControl;
use super::transitions::WorkflowRunTransitions;
use crate::agent_runtime::AgentRuntimeManager;
use crate::clock::SystemClock;
use crate::error::{BackendError, ErrorClassification};
use crate::git_cleanup::KeyedResourceLocks;
use crate::repository_work::spawn_repository_work;
use ora_application::{ApplicationError, Clock, WorkflowRunEngineRepository};
use ora_contracts::*;
use ora_db::{RepositoryPool, SqliteWorkflowRunEngineRepository};
use ora_logging::{ora_info, ora_warn};
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
    pub transitions: Arc<WorkflowRunTransitions>,
    pub plugin_api: Arc<crate::plugin::PluginApi>,
    pub clock: SystemClock,
    /// E2E-only owner-stream drop control shared with the node executor.
    #[cfg(feature = "test-support")]
    pub owner_stream_drop: Arc<OwnerStreamDropControl>,
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
    skills_root: PathBuf,
    agent_runtime: Arc<AgentRuntimeManager>,
    engine: Arc<ConcreteWorkflowRunControl>,
    run_locks: Arc<KeyedResourceLocks>,
    completing_node_runs: Arc<CompletingNodeRuns>,
    transitions: Arc<WorkflowRunTransitions>,
    /// E2E-only owner-stream drop control shared with the node executor.
    #[cfg(feature = "test-support")]
    owner_stream_drop: Arc<OwnerStreamDropControl>,
}

impl WorkflowRuns {
    /// Captures the instances composed at startup; no lock, worker, or supervisor is restarted.
    pub(crate) fn new(setup: WorkflowRunSetup) -> Self {
        Self {
            records: WorkflowRunApi::new(
                setup.pool.clone(),
                setup.skills_root.clone(),
                setup.plugin_api.clone(),
                setup.clock,
            ),
            pool: setup.pool,
            sessions_root: setup.sessions_root,
            baselines_root: setup.baselines_root,
            skills_root: setup.skills_root,
            agent_runtime: setup.agent_runtime,
            engine: setup.engine,
            run_locks: setup.run_locks,
            completing_node_runs: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            transitions: setup.transitions,
            #[cfg(feature = "test-support")]
            owner_stream_drop: setup.owner_stream_drop,
        }
    }

    /// Arms one node whose real owning prompt stream an E2E test will abandon.
    #[cfg(feature = "test-support")]
    pub fn arm_test_owner_stream_drop(&self, node_id: &str) {
        self.owner_stream_drop.arm(node_id);
    }

    /// Waits for the executor to claim the armed node and returns its real session id.
    #[cfg(feature = "test-support")]
    pub async fn wait_for_test_owner_stream_session(&self) -> Option<String> {
        self.owner_stream_drop.wait_for_claimed_session().await
    }

    /// Confirms prompt receipt for the claimed session and requests the owner drop.
    #[cfg(feature = "test-support")]
    pub fn confirm_test_owner_stream_prompt(&self, session_id: &str) -> Result<(), String> {
        self.owner_stream_drop.confirm_prompt_received(session_id)
    }

    /// Grants Sessions only the admission/cleanup capability sharing this run's coordination.
    pub(crate) fn session_turns(&self) -> WorkflowSessionTurns {
        WorkflowSessionTurns::new(
            self.pool.clone(),
            self.run_locks.clone(),
            self.completing_node_runs.clone(),
            self.transitions.clone(),
            self.sessions_root.clone(),
            self.baselines_root.clone(),
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
        let confirm_initial_turn = request.confirm_initial_turn.unwrap_or(false);
        // The two approvals contradict each other: one accepts the completion gate, the other
        // overrides it. Reject before claiming the node so a contradictory request changes no
        // state (ADR D2 row 1).
        if request.force && confirm_initial_turn {
            return Err(BackendError::new(
                ErrorClassification::InvalidRequest,
                PublicError::InvalidRequest(EmptyErrorParams {}),
                "force and confirmInitialTurn cannot be combined",
            ));
        }
        let approval = if request.force {
            CompletionApproval::FORCE
        } else if confirm_initial_turn {
            CompletionApproval::CONFIRM
        } else {
            CompletionApproval::PLAIN
        };
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
        // node returns to its awaitable state. The worktree diff needs the runtime's workspace
        // directory, so it is resolved here on the same blocking worker as the prepare itself.
        let (prepared, file_changes) = {
            let pool = self.pool.clone();
            let sessions_root = self.sessions_root.clone();
            let baselines_root = self.baselines_root.clone();
            let agent_runtime = self.agent_runtime.clone();
            let run_id = run_id.clone();
            let node_id = node_id.clone();
            match spawn_repository_work(move || {
                // Resolve the run workspace before preparing: the file gate reads the same
                // workspace the node ran in, and both reads happen outside the per-run gate.
                let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
                let workspace_id = repository
                    .find_execution_context(&run_id)
                    .map_err(|source| {
                        BackendError::from(ApplicationError::WorkflowRunRepository { source })
                    })?
                    .ok_or_else(|| {
                        BackendError::from(ApplicationError::WorkflowRunNotFound {
                            run_id: run_id.to_string(),
                        })
                    })?
                    .workspace
                    .id;
                let worktree_root = agent_runtime.workspace_cwd(&workspace_id)?;
                let prepared = crate::workflow::run::interactive::prepare_completion(
                    &pool,
                    &sessions_root,
                    &baselines_root,
                    &worktree_root,
                    &run_id,
                    &node_id,
                    approval,
                )?;
                let file_changes = crate::workflow::run::interactive::completion_file_changes(
                    &baselines_root,
                    &prepared.node_run_id,
                    &worktree_root,
                    prepared.completion.as_ref(),
                );
                Ok((prepared, file_changes))
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
        // A forced completion merges its audit in that same transaction, so a committed node always
        // carries the record of how it was completed (ADR D5).
        let pool = self.pool.clone();
        let engine = self.engine.clone();
        let run_locks = self.run_locks.clone();
        let completing = self.completing_node_runs.clone();
        let node_run_id = prepared.node_run_id.clone();
        let output = prepared.output.clone();
        let structured_output = prepared.structured_output.clone();
        let stop_reason = prepared.stop_reason.clone();
        let audit = prepared.audit.clone();
        let response = spawn_repository_work(move || {
            let _gate = run_locks.acquire_exclusive(run_id.as_ref());
            let result = crate::workflow::run::interactive::revalidate_completion(
                &pool,
                &run_id,
                &node_run_id,
            )
            .and_then(|()| {
                engine
                    .complete_node_with_payload_patch(
                        &run_id,
                        &node_run_id,
                        output,
                        structured_output,
                        stop_reason,
                        file_changes,
                        audit,
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

    /// Resumes a failed or cancelled workflow run from its failed nodes.
    pub fn resume_from_failure(
        &self,
        request: ResumeWorkflowRunRequest,
    ) -> Result<ResumeWorkflowRunResponse, BackendError> {
        let _gate = self.run_locks.acquire_exclusive(request.run_id.clone());
        super::rollback::resume_from_failure(
            &self.pool,
            &self.agent_runtime,
            &self.skills_root,
            &self.engine,
            &self.transitions,
            request,
            SystemClock.now_timestamp_millis(),
        )
    }

    /// Reads rollback availability and live worktree diffs without mutating the run.
    pub fn preview_resume(
        &self,
        request: PreviewWorkflowRunResumeRequest,
    ) -> Result<PreviewWorkflowRunResumeResponse, BackendError> {
        let _gate = self.run_locks.acquire_shared(request.run_id.clone());
        super::rollback::preview_resume(&self.pool, &self.agent_runtime, request)
    }

    /// Asks the node's own agent to guess why it failed. The answer is stored as provenance only.
    pub async fn diagnose_node_failure(
        &self,
        request: DiagnoseWorkflowNodeFailureRequest,
    ) -> Result<DiagnoseWorkflowNodeFailureResponse, BackendError> {
        let run_id = request.run_id.clone();
        let node_id = request.node_id.clone();
        let pool = self.pool.clone();
        let sessions_root = self.sessions_root.clone();
        let run_locks = self.run_locks.clone();
        let (input, executor, workspace_id, node_run_id) = spawn_repository_work({
            let run_id = run_id.clone();
            move || {
                let _gate = run_locks.acquire_shared(run_id.clone());
                super::diagnosis::load_input(&pool, &sessions_root, &run_id, &node_id)
            }
        })
        .await?;
        let prompt = super::diagnosis::build_diagnosis_prompt(&input);
        let text =
            super::diagnosis::run_diagnosis(&self.agent_runtime, workspace_id, &executor, prompt)
                .await?;
        let generated_at = SystemClock.now_timestamp_millis();
        let diagnosis = WorkflowNodeAiDiagnosis {
            text,
            agent_cli: executor.agent_cli.clone(),
            model: executor.model_id.clone(),
            generated_at,
        };
        let diagnosis_json = serde_json::json!({
            "text": diagnosis.text,
            "agent_cli": diagnosis.agent_cli,
            "model": diagnosis.model,
            "generated_at": diagnosis.generated_at,
        })
        .to_string();
        let pool = self.pool.clone();
        let run_locks = self.run_locks.clone();
        spawn_repository_work(move || {
            let _gate = run_locks.acquire_shared(run_id.clone());
            SqliteWorkflowRunEngineRepository::new(pool)
                .record_node_ai_diagnosis(&node_run_id, &diagnosis_json, generated_at)
                .map_err(|source| {
                    BackendError::from(ApplicationError::WorkflowRunRepository { source })
                })
        })
        .await?;
        ora_info!(
            run_id = %request.run_id,
            node_id = %request.node_id,
            agent = %diagnosis.agent_cli,
            model = %diagnosis.model,
            text_len = diagnosis.text.chars().count(),
            "recorded workflow node AI diagnosis"
        );
        Ok(DiagnoseWorkflowNodeFailureResponse { diagnosis })
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
