use crate::agent::AgentApi;
use crate::agent_runtime::{AgentRuntimeManager, SessionEventStream, SessionLocator};
use crate::app_event::AppEventHub;
use crate::clock::SystemClock;
use crate::error::{BackendError, ErrorClassification};
use crate::plugin::PluginApi;
use crate::project::ProjectApi;
use crate::session::SessionApi;
use crate::skill::SkillApi;
use crate::spec::SpecApi;
use crate::task::TaskApi;
use crate::task_diff::TaskDiffApi;
use crate::workflow::WorkflowApi;
use crate::workflow_run::WorkflowRunApi;
use crate::workflow_run_engine::{ConcreteWorkflowRunControl, build_workflow_run_engine};
use ora_application::{ApplicationError, Clock, WorkflowRunEngineRepository};
use ora_contracts::*;
use ora_contracts::{EmptyErrorParams, PublicError};
use ora_db::SqliteWorkflowRunEngineRepository;
use ora_db::{DatabaseBootstrapper, DatabaseLocation, RepositoryPool, default_migration_catalog};
use ora_logging::{ora_error, ora_warn};
use ora_scheduler::Scheduler;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use thiserror::Error;

/// Names the persistent paths required to construct the shared backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendPaths {
    pub database_path: PathBuf,
    /// Root containing the installed `plugins` directory.
    pub data_directory: PathBuf,
    /// Bundled Deno executable used for plugin activation.
    pub deno_path: PathBuf,
    pub worktree_root: PathBuf,
    pub home_directory: PathBuf,
    /// Directory against which persisted relative project roots are resolved.
    ///
    /// Relative roots are stored against the directory from which `ORA_DATA_DIR`
    /// was created. Live process cwd is not used: Desktop `tauri dev` starts in
    /// `src-tauri`, which is not that directory.
    pub relative_path_base: PathBuf,
    pub sessions_root: PathBuf,
    /// Root of the formal skill package tree (`<data>/atoms/skills`).
    pub skills_root: PathBuf,
    /// Bundled ripgrep executable used by shared specification discovery.
    pub ripgrep_path: PathBuf,
    /// IANA timezone used by backend-owned cron and delayed work.
    pub timezone: chrono_tz::Tz,
}

/// Reports failures that prevent the shared backend from opening persistent state.
#[derive(Debug, Error)]
pub enum BackendBootstrapError {
    #[error("failed to create backend directory {path:?}")]
    DirectoryCreate {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to bootstrap backend database")]
    Database(#[source] ora_db::DatabaseError),
    #[error("failed to initialize plugin lifecycle")]
    PluginLifecycle(#[source] ora_plugin_lifecycle::PluginLifecycleError),
    #[error("failed to reconcile skill storage")]
    SkillStorage(#[source] ApplicationError),
    #[error("failed to initialize agent runtime")]
    AgentRuntime(#[source] BackendError),
    #[error("failed to reconcile skill storage")]
    SkillStorageReconciliation(
        #[source] crate::skill_reconciliation::SkillStorageReconciliationError,
    ),
}

/// Owns the concrete persisted use-case composition used by the Desktop adapter.
#[derive(Clone)]
pub struct Backend {
    pool: RepositoryPool,
    worktree_root: Arc<RwLock<PathBuf>>,
    project: Arc<ProjectApi>,
    task: Arc<TaskApi>,
    task_diff: Arc<TaskDiffApi>,
    session: Arc<SessionApi>,
    agent_runtime: Arc<AgentRuntimeManager>,
    plugin: Arc<PluginApi>,
    skill: Arc<SkillApi>,
    agent: Arc<AgentApi>,
    spec: Arc<SpecApi>,
    workflow: Arc<WorkflowApi>,
    workflow_run: Arc<WorkflowRunApi>,
    workflow_run_engine: Arc<ConcreteWorkflowRunControl>,
    app_events: Arc<AppEventHub>,
    git_cleanup: crate::git_cleanup::GitCleanupHandle,
    relative_path_base: PathBuf,
}

impl Backend {
    /// Opens persistent storage and constructs every shared CRUD API.
    pub fn open(paths: BackendPaths) -> Result<Self, BackendBootstrapError> {
        ensure_directory(
            paths
                .database_path
                .parent()
                .unwrap_or_else(|| Path::new(".")),
        )?;
        ensure_directory(&paths.worktree_root)?;
        let catalog = default_migration_catalog().map_err(BackendBootstrapError::Database)?;
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(&DatabaseLocation::path(&paths.database_path), &catalog)
            .map_err(BackendBootstrapError::Database)?;
        crate::skill_reconciliation::reconcile_skill_storage(&pool, &paths.skills_root)
            .map_err(BackendBootstrapError::SkillStorageReconciliation)?;
        crate::skill_reconciliation::cleanup_import_temp_sessions()
            .map_err(BackendBootstrapError::SkillStorageReconciliation)?;
        let clock = SystemClock;
        let app_events = Arc::new(AppEventHub::new());
        let plugin = Arc::new(
            PluginApi::open(
                pool.clone(),
                paths.data_directory,
                paths.deno_path,
                clock,
                app_events.publisher(),
            )
            .map_err(BackendBootstrapError::PluginLifecycle)?,
        );
        let scheduler = Scheduler::new(paths.timezone);
        let worktree_root = Arc::new(RwLock::new(paths.worktree_root));
        let sessions_root = paths.sessions_root;
        let relative_path_base = paths.relative_path_base;
        let agent_runtime = Arc::new(
            AgentRuntimeManager::new(
                pool.clone(),
                paths.home_directory,
                relative_path_base.clone(),
                sessions_root.clone(),
                clock,
                scheduler,
                app_events.publisher(),
            )
            .map_err(BackendBootstrapError::AgentRuntime)?,
        );
        // Crash recovery: fail orphaned node runs and running runs left by a previous process
        // before serving new commands (best-effort; a failure must not block startup).
        run_workflow_run_boot_sweep(&pool, clock);
        // Durable Git cleanup: the worker's first pass replays every cleanup job
        // and expired provisioning lease a previous process left behind.
        let git_cleanup_worker =
            crate::git_cleanup::GitCleanupWorker::new(pool.clone(), worktree_root.clone(), clock);
        let repository_gates = git_cleanup_worker.repository_gates();
        let git_cleanup = git_cleanup_worker.spawn();

        let workflow_run_engine = build_workflow_run_engine(
            agent_runtime.clone(),
            pool.clone(),
            paths.skills_root.clone(),
            clock,
        )
        .control;

        Ok(Self {
            project: Arc::new(ProjectApi::new(pool.clone(), sessions_root.clone(), clock)),
            task: Arc::new(TaskApi::new(
                pool.clone(),
                worktree_root.clone(),
                sessions_root,
                repository_gates.clone(),
                clock,
            )),
            task_diff: Arc::new(TaskDiffApi::new(
                pool.clone(),
                clock,
                git_cleanup.clone(),
                relative_path_base.clone(),
            )),
            session: Arc::new(SessionApi::new(pool.clone())),
            agent_runtime,
            plugin,
            skill: Arc::new(SkillApi::new(
                pool.clone(),
                paths.skills_root.clone(),
                clock,
            )),
            agent: Arc::new(AgentApi::new(pool.clone(), clock)),
            spec: Arc::new(SpecApi::new(
                pool.clone(),
                paths.ripgrep_path,
                git_cleanup.clone(),
                relative_path_base.clone(),
            )),
            workflow: Arc::new(WorkflowApi::new(pool.clone(), clock)),
            workflow_run: Arc::new(WorkflowRunApi::new(
                pool.clone(),
                worktree_root.clone(),
                paths.skills_root,
                repository_gates,
                clock,
            )),
            workflow_run_engine,
            app_events,
            git_cleanup,
            pool,
            worktree_root,
            relative_path_base,
        })
    }

    /// Returns the cached installed-plugin snapshot without rescanning the filesystem.
    pub fn list_installed_plugins(
        &self,
        request: ListInstalledPluginsRequest,
    ) -> Result<ListInstalledPluginsResponse, BackendError> {
        Ok(self.plugin.list(request))
    }

    /// Explicitly rescans packages and reconciles durable and runtime state.
    pub async fn scan_plugins(
        &self,
        request: ScanPluginsRequest,
    ) -> Result<ScanPluginsResponse, BackendError> {
        self.plugin.scan(request).await.map_err(BackendError::from)
    }

    /// Persists plugin eligibility without starting its process.
    pub async fn enable_plugin(
        &self,
        request: EnablePluginRequest,
    ) -> Result<EnablePluginResponse, BackendError> {
        self.plugin
            .enable(request)
            .await
            .map_err(BackendError::from)
    }

    /// Stops a plugin when necessary before persisting ineligibility.
    pub async fn disable_plugin(
        &self,
        request: DisablePluginRequest,
    ) -> Result<DisablePluginResponse, BackendError> {
        self.plugin
            .disable(request)
            .await
            .map_err(BackendError::from)
    }

    /// Starts one enabled plugin and returns its immediate starting state.
    pub async fn activate_plugin(
        &self,
        request: ActivatePluginRequest,
    ) -> Result<ActivatePluginResponse, BackendError> {
        self.plugin
            .activate(request)
            .await
            .map_err(BackendError::from)
    }

    /// Stops one plugin process without changing durable eligibility.
    pub async fn stop_plugin(
        &self,
        request: StopPluginRequest,
    ) -> Result<StopPluginResponse, BackendError> {
        self.plugin.stop(request).await.map_err(BackendError::from)
    }

    /// Stops and removes one plugin package plus its durable state.
    pub async fn uninstall_plugin(
        &self,
        request: UninstallPluginRequest,
    ) -> Result<UninstallPluginResponse, BackendError> {
        self.plugin
            .uninstall(request)
            .await
            .map_err(BackendError::from)
    }

    /// Starts a workflow run against its frozen snapshot graph.
    pub fn start_workflow_run(
        &self,
        request: StartWorkflowRunRequest,
    ) -> Result<StartWorkflowRunResponse, BackendError> {
        self.workflow_run_engine
            .start(request)
            .map_err(BackendError::from)
    }

    /// Cancels a running workflow run and stops its live node sessions.
    ///
    /// The engine commits the `Cancelled` transition first; then every session still bound to the
    /// run's node runs is stopped. Without this second step the agent keeps executing its prompt
    /// and the delete guard treats the lingering `Running` session as an active run.
    pub async fn cancel_workflow_run(
        &self,
        request: CancelWorkflowRunRequest,
    ) -> Result<CancelWorkflowRunResponse, BackendError> {
        let run_id = ora_domain::WorkflowRunId::new(&request.run_id);
        let engine = self.workflow_run_engine.clone();
        let response =
            spawn_repository_work(move || engine.cancel(request).map_err(BackendError::from))
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

    /// Restarts a finished workflow run.
    pub fn restart_workflow_run(
        &self,
        request: RestartWorkflowRunRequest,
    ) -> Result<RestartWorkflowRunResponse, BackendError> {
        self.workflow_run_engine
            .restart(request)
            .map_err(BackendError::from)
    }

    /// Sets the kickoff input of a pending workflow run.
    pub fn update_workflow_run_input(
        &self,
        request: UpdateWorkflowRunInputRequest,
    ) -> Result<UpdateWorkflowRunInputResponse, BackendError> {
        self.workflow_run_engine
            .update_input(request)
            .map_err(BackendError::from)
    }

    /// Returns the repository pool needed by server-only services excluded from this extraction.
    pub fn repository_pool(&self) -> RepositoryPool {
        self.pool.clone()
    }

    /// Replaces the root used by task creations that start after this update.
    pub fn set_worktree_root(&self, worktree_root: PathBuf) -> Result<(), BackendError> {
        let mut configured_root = self.worktree_root.write().map_err(|_poisoned| {
            BackendError::new(
                ErrorClassification::Internal,
                PublicError::InternalError(EmptyErrorParams {}),
                "worktree root configuration is unavailable",
            )
        })?;
        *configured_root = worktree_root;
        Ok(())
    }

    /// Resolves the on-disk git worktree directory that backs one task.
    ///
    /// Reuses the same live resolution the agent runtime performs before spawning a
    /// provider, so the path always matches where the session actually runs. Fails
    /// when the task has no active worktree on disk.
    pub fn resolve_task_cwd(&self, task_id: &str) -> Result<PathBuf, BackendError> {
        crate::task::resolve_task_cwd(
            &self.pool,
            &ora_domain::TaskId::new(task_id),
            &self.relative_path_base,
        )
    }

    // =============================================================================
    // project
    // =============================================================================

    /// Creates one project through the shared application composition.
    pub fn create_project(
        &self,
        request: CreateProjectRequest,
    ) -> Result<CreateProjectResponse, BackendError> {
        self.project.create(request).map_err(BackendError::from)
    }
    /// Gets one project through the shared application composition.
    pub fn get_project(
        &self,
        request: GetProjectRequest,
    ) -> Result<GetProjectResponse, BackendError> {
        self.project.get(request).map_err(BackendError::from)
    }
    /// Lists projects through the shared application composition.
    pub fn list_projects(
        &self,
        request: ListProjectsRequest,
    ) -> Result<ListProjectsResponse, BackendError> {
        self.project.list(request).map_err(BackendError::from)
    }
    /// Lists selectable branches for one project repository.
    pub fn list_project_branches(
        &self,
        request: ListProjectBranchesRequest,
    ) -> Result<ListProjectBranchesResponse, BackendError> {
        self.project
            .list_branches(request)
            .map_err(BackendError::from)
    }
    /// Updates one project through the shared application composition.
    pub fn update_project(
        &self,
        request: UpdateProjectRequest,
    ) -> Result<UpdateProjectResponse, BackendError> {
        self.project.update(request).map_err(BackendError::from)
    }
    /// Deletes one project through the shared application composition.
    ///
    /// The delete cascades to the project's Tasks, so every chat surface beneath
    /// it dies along with the project root's and their warm sessions are
    /// discarded together. The Task identifiers are collected first because the
    /// delete is what makes those rows invisible; both queries share one blocking
    /// hop so that ordering cannot be broken by a scheduling decision.
    pub async fn delete_project(
        &self,
        request: DeleteProjectRequest,
    ) -> Result<DeleteProjectResponse, BackendError> {
        let project = self.project.clone();
        let pool = self.pool.clone();
        let project_id = ora_domain::ProjectId::new(request.project_id.as_str());
        let (response, task_ids) = spawn_repository_work(move || {
            let task_ids = crate::task::task_ids_in_project(&pool, &project_id);
            project.delete(request).map(|response| (response, task_ids))
        })
        .await?;

        let mut targets: Vec<WarmSessionTarget> = task_ids
            .into_iter()
            .map(|task_id| WarmSessionTarget::Task {
                task_id: task_id.to_string(),
            })
            .collect();
        targets.push(WarmSessionTarget::ProjectRoot {
            project_id: response.project_id.clone(),
        });
        self.agent_runtime.discard_warm_sessions(&targets).await;
        // The cascade registered the cleanup jobs; this only trims their latency.
        self.git_cleanup.notify();
        Ok(response)
    }

    // =============================================================================
    // task
    // =============================================================================

    /// Creates one task through the shared application composition.
    pub fn create_task(
        &self,
        request: CreateTaskRequest,
    ) -> Result<CreateTaskResponse, BackendError> {
        self.task.create(request).map_err(BackendError::from)
    }
    /// Gets one task through the shared application composition.
    pub fn get_task(&self, request: GetTaskRequest) -> Result<GetTaskResponse, BackendError> {
        self.task.get(request).map_err(BackendError::from)
    }
    /// Lists tasks through the shared application composition.
    pub fn list_tasks(&self, request: ListTasksRequest) -> Result<ListTasksResponse, BackendError> {
        self.task.list(request).map_err(BackendError::from)
    }
    /// Updates one task through the shared application composition.
    pub fn update_task(
        &self,
        request: UpdateTaskRequest,
    ) -> Result<UpdateTaskResponse, BackendError> {
        self.task.update(request).map_err(BackendError::from)
    }
    /// Deletes one task through the shared application composition.
    ///
    /// Discards the Task's warm session on the way out. Nothing else would: the
    /// target is gone, so no request can name it again, and the pool's bounds
    /// only reclaim an entry once enough new surfaces are opened to displace it.
    pub async fn delete_task(
        &self,
        request: DeleteTaskRequest,
    ) -> Result<DeleteTaskResponse, BackendError> {
        let task = self.task.clone();
        let response = spawn_repository_work(move || task.delete(request)).await?;
        self.agent_runtime
            .discard_warm_sessions(&[WarmSessionTarget::Task {
                task_id: response.task_id.clone(),
            }])
            .await;
        // The cascade registered the cleanup job; this only trims its latency.
        self.git_cleanup.notify();
        Ok(response)
    }

    /// Returns the authoritative checkout root and optional branch for a task.
    pub fn get_task_workspace(
        &self,
        request: GetTaskWorkspaceRequest,
    ) -> Result<GetTaskWorkspaceResponse, BackendError> {
        crate::task::get_task_workspace(&self.pool, &request.task_id, &self.relative_path_base)
    }

    // =============================================================================
    // spec
    // =============================================================================

    /// Builds the effective specification catalog for one project or task target.
    pub async fn get_spec_catalog(
        &self,
        request: GetSpecCatalogRequest,
    ) -> Result<SpecCatalogResponse, BackendError> {
        self.spec.catalog(request).await
    }

    /// Reads one catalog-authorized Markdown document.
    pub async fn read_spec(
        &self,
        request: ReadSpecRequest,
    ) -> Result<ReadSpecResponse, BackendError> {
        self.spec.read(request).await
    }

    /// Resolves a specification watch request to its authoritative workspace root.
    pub fn resolve_spec_watch_root(
        &self,
        request: &WatchSpecsRequest,
    ) -> Result<PathBuf, BackendError> {
        self.spec.watch_root(&request.target)
    }

    // =============================================================================
    // taskDiff
    // =============================================================================
    /// Returns the current Git snapshot for the task directory used by its agent session.
    pub fn get_task_diff(
        &self,
        request: GetTaskDiffRequest,
    ) -> Result<GetTaskDiffResponse, BackendError> {
        self.task_diff.get_diff(request)
    }

    /// Commits every current change in one isolated task worktree.
    pub fn commit_task_changes(
        &self,
        request: CommitTaskChangesRequest,
    ) -> Result<CommitTaskChangesResponse, BackendError> {
        self.task_diff.commit_changes(request)
    }

    /// Pushes the verified branch owned by one isolated task worktree.
    pub fn push_task_branch(
        &self,
        request: PushTaskBranchRequest,
    ) -> Result<PushTaskBranchResponse, BackendError> {
        self.task_diff.push_branch(request)
    }

    /// Lists every persisted review discussion for one task.
    pub fn list_task_diff_comments(
        &self,
        request: ListTaskDiffCommentsRequest,
    ) -> Result<ListTaskDiffCommentsResponse, BackendError> {
        self.task_diff.list_comments(request)
    }

    /// Creates one line-anchored task diff discussion.
    pub fn create_task_diff_comment(
        &self,
        request: CreateTaskDiffCommentRequest,
    ) -> Result<CreateTaskDiffCommentResponse, BackendError> {
        self.task_diff.create_comment(request)
    }

    /// Adds one reply under an existing task diff discussion.
    pub fn reply_task_diff_comment(
        &self,
        request: ReplyTaskDiffCommentRequest,
    ) -> Result<ReplyTaskDiffCommentResponse, BackendError> {
        self.task_diff.reply_comment(request)
    }

    /// Resolves or reopens one root task diff discussion.
    pub fn set_task_diff_comment_status(
        &self,
        request: SetTaskDiffCommentStatusRequest,
    ) -> Result<SetTaskDiffCommentStatusResponse, BackendError> {
        self.task_diff.set_comment_status(request)
    }

    // =============================================================================
    // session
    // =============================================================================

    /// Returns the warm provider session backing one chat surface.
    pub async fn warm_session(
        &self,
        request: WarmSessionRequest,
    ) -> Result<WarmSessionResponse, BackendError> {
        self.agent_runtime.warm_session(request).await
    }

    /// Applies one configuration option to a warm or persisted session.
    pub async fn set_session_config(
        &self,
        request: SetSessionConfigRequest,
    ) -> Result<SetSessionConfigResponse, BackendError> {
        self.agent_runtime.set_session_config(request).await
    }

    /// Persists one warm session against the Task that now owns it.
    pub async fn attach_session(
        &self,
        request: AttachSessionRequest,
    ) -> Result<AttachSessionResponse, BackendError> {
        self.agent_runtime.attach_session(request).await
    }

    /// Gets one session through the shared application composition.
    pub fn get_session(
        &self,
        request: GetSessionRequest,
    ) -> Result<GetSessionResponse, BackendError> {
        self.session.get(request).map_err(BackendError::from)
    }
    /// Lists sessions through the shared application composition.
    pub fn list_sessions(
        &self,
        request: ListSessionsRequest,
    ) -> Result<ListSessionsResponse, BackendError> {
        self.session.list(request).map_err(BackendError::from)
    }

    /// Renames one session, locks agent title acquisition, then notifies subscribers.
    pub async fn rename_session(
        &self,
        request: RenameSessionRequest,
    ) -> Result<RenameSessionResponse, BackendError> {
        let session_id = request.session_id.clone();
        let response = self.session.rename(request).map_err(BackendError::from)?;
        if let Some(title) = response.session.title.as_deref()
            && let Ok(parsed) = ora_domain::SessionTitle::parse(title)
        {
            // A missing or busy actor must not fail the rename: the row is already updated.
            let _ = self
                .agent_runtime
                .adopt_user_title(&session_id, parsed)
                .await;
        }
        self.app_events
            .publisher()
            .try_publish(AppEvent::SessionTitleUpdated { session_id });
        Ok(response)
    }
    /// Streams the provider-owned history for one persisted session.
    pub async fn load_session(
        &self,
        request: LoadSessionRequest,
    ) -> Result<SessionEventStream<LoadSessionEvent>, BackendError> {
        self.agent_runtime.load_session(request).await
    }

    /// Opens one subscriber to the shared application event stream.
    pub fn watch_app_events(&self) -> SessionEventStream<AppEvent> {
        self.app_events.subscribe()
    }

    /// Streams one structured ACP prompt turn for a running session.
    pub async fn prompt_session(
        &self,
        request: PromptSessionRequest,
    ) -> Result<SessionEventStream<PromptSessionEvent>, BackendError> {
        self.agent_runtime.prompt_session(request).await
    }

    /// Delivers one validated permission response to the owning session actor.
    pub async fn respond_to_session_permission(
        &self,
        request: RespondToPermissionRequest,
    ) -> Result<RespondToPermissionResponse, BackendError> {
        self.agent_runtime.respond_to_permission(request).await
    }

    /// Unloads one running session while retaining its provider history and Ora record.
    pub async fn stop_session(
        &self,
        request: StopSessionRequest,
    ) -> Result<StopSessionResponse, BackendError> {
        self.agent_runtime.stop_session(request).await
    }

    /// Moves one existing conversation onto a different agent CLI.
    pub async fn switch_session_agent(
        &self,
        request: SwitchSessionAgentRequest,
    ) -> Result<SwitchSessionAgentResponse, BackendError> {
        self.agent_runtime.switch_agent(request).await
    }

    /// Returns a session whose history writes failed to a writable state.
    pub async fn resume_session_history(
        &self,
        request: ResumeSessionHistoryRequest,
    ) -> Result<ResumeSessionHistoryResponse, BackendError> {
        self.agent_runtime.resume_history(request).await
    }

    /// Stops one session before removing its Ora-owned record and recorded history.
    pub async fn delete_session(
        &self,
        request: DeleteSessionRequest,
    ) -> Result<DeleteSessionResponse, BackendError> {
        self.agent_runtime.delete_session(&request.session_id).await
    }

    // =============================================================================
    // agentRuntime
    // =============================================================================

    /// Reports whether each application-scoped CLI runtime is ready, starting, or unavailable.
    pub fn get_agent_runtime_status(
        &self,
        _request: GetAgentRuntimeStatusRequest,
    ) -> Result<GetAgentRuntimeStatusResponse, BackendError> {
        Ok(self.agent_runtime.agent_runtime_status())
    }

    /// Resolves one Ora session id to its private agent session identifier and worktree cwd.
    ///
    /// Backend-only: the returned `agent_session_id` is never exposed to the frontend. The
    /// Desktop dashboard command consumes it to locate the agent-written trace file.
    pub fn resolve_session_locator(
        &self,
        session_id: &str,
    ) -> Result<SessionLocator, BackendError> {
        self.agent_runtime.resolve_session_locator(session_id)
    }

    // =============================================================================
    // skill
    // =============================================================================

    /// Creates one skill through the shared application composition.
    pub fn create_skill(
        &self,
        request: CreateSkillRequest,
    ) -> Result<CreateSkillResponse, BackendError> {
        self.skill.create(request).map_err(BackendError::from)
    }
    /// Gets one skill through the shared application composition.
    pub fn get_skill(&self, request: GetSkillRequest) -> Result<GetSkillResponse, BackendError> {
        self.skill.get(request).map_err(BackendError::from)
    }
    /// Lists skills through the shared application composition.
    pub fn list_skills(
        &self,
        request: ListSkillsRequest,
    ) -> Result<ListSkillsResponse, BackendError> {
        self.skill.list(request).map_err(BackendError::from)
    }
    /// Updates one skill through the shared application composition.
    pub fn update_skill(
        &self,
        request: UpdateSkillRequest,
    ) -> Result<UpdateSkillResponse, BackendError> {
        self.skill.update(request).map_err(BackendError::from)
    }
    /// Deletes one skill through the shared application composition.
    pub fn delete_skill(
        &self,
        request: DeleteSkillRequest,
    ) -> Result<DeleteSkillResponse, BackendError> {
        self.skill.delete(request).map_err(BackendError::from)
    }
    // =============================================================================
    // agent
    // =============================================================================

    /// Prepares one skill import source into a previewed session.
    pub fn prepare_skill_import(
        &self,
        request: PrepareSkillImportRequest,
    ) -> Result<PrepareSkillImportResponse, BackendError> {
        self.skill
            .prepare_import(request)
            .map_err(BackendError::from)
    }
    /// Returns one skill import session with its current progress.
    pub fn get_skill_import(
        &self,
        request: GetSkillImportSessionRequest,
    ) -> Result<GetSkillImportSessionResponse, BackendError> {
        self.skill.get_import(request).map_err(BackendError::from)
    }
    /// Accepts and freezes one skill import commit, starting the background task.
    pub fn commit_skill_import(
        &self,
        request: CommitSkillImportRequest,
    ) -> Result<CommitSkillImportResponse, BackendError> {
        self.skill
            .commit_import(request)
            .map_err(BackendError::from)
    }
    /// Cancels a prepared skill import session.
    pub fn cancel_skill_import(
        &self,
        request: CancelSkillImportRequest,
    ) -> Result<CancelSkillImportResponse, BackendError> {
        self.skill
            .cancel_import(request)
            .map_err(BackendError::from)
    }

    /// Creates one configurable agent through the shared application composition.
    pub fn create_agent(
        &self,
        request: CreateAgentRequest,
    ) -> Result<CreateAgentResponse, BackendError> {
        self.agent.create(request).map_err(BackendError::from)
    }
    /// Gets one configurable agent through the shared application composition.
    pub fn get_agent(&self, request: GetAgentRequest) -> Result<GetAgentResponse, BackendError> {
        self.agent.get(request).map_err(BackendError::from)
    }
    /// Lists configurable agents through the shared application composition.
    pub fn list_agents(
        &self,
        request: ListAgentsRequest,
    ) -> Result<ListAgentsResponse, BackendError> {
        self.agent.list(request).map_err(BackendError::from)
    }
    /// Updates one configurable agent through the shared application composition.
    pub fn update_agent(
        &self,
        request: UpdateAgentRequest,
    ) -> Result<UpdateAgentResponse, BackendError> {
        self.agent.update(request).map_err(BackendError::from)
    }
    /// Deletes one configurable agent through the shared application composition.
    pub fn delete_agent(
        &self,
        request: DeleteAgentRequest,
    ) -> Result<DeleteAgentResponse, BackendError> {
        self.agent.delete(request).map_err(BackendError::from)
    }
    pub fn prepare_agent_import(
        &self,
        request: PrepareAgentImportRequest,
    ) -> Result<PrepareAgentImportResponse, BackendError> {
        self.agent
            .prepare_import(request)
            .map_err(BackendError::from)
    }

    pub fn commit_agent_import(
        &self,
        request: CommitAgentImportRequest,
    ) -> Result<CommitAgentImportResponse, BackendError> {
        self.agent
            .commit_import(request)
            .map_err(BackendError::from)
    }

    // =============================================================================
    // gitIdentity
    // =============================================================================

    /// Reads the host identity for the sidebar profile: global git config first,
    /// falling back to the authenticated GitHub CLI account when git has no name set.
    pub fn read_git_identity(
        &self,
        _request: GetGitIdentityRequest,
    ) -> Result<GitIdentityResponse, BackendError> {
        Ok(crate::identity::resolve_git_identity())
    }

    // =============================================================================
    // workflow
    // =============================================================================

    /// Creates one workflow through the shared application composition.
    pub fn create_workflow(
        &self,
        request: CreateWorkflowRequest,
    ) -> Result<CreateWorkflowResponse, BackendError> {
        self.workflow.create(request).map_err(BackendError::from)
    }
    /// Gets one workflow through the shared application composition.
    pub fn get_workflow(
        &self,
        request: GetWorkflowRequest,
    ) -> Result<GetWorkflowResponse, BackendError> {
        self.workflow.get(request).map_err(BackendError::from)
    }
    /// Lists workflows through the shared application composition.
    pub fn list_workflows(
        &self,
        request: ListWorkflowsRequest,
    ) -> Result<ListWorkflowsResponse, BackendError> {
        self.workflow.list(request).map_err(BackendError::from)
    }
    /// Updates one workflow through the shared application composition.
    pub fn update_workflow(
        &self,
        request: UpdateWorkflowRequest,
    ) -> Result<UpdateWorkflowResponse, BackendError> {
        self.workflow.update(request).map_err(BackendError::from)
    }
    /// Deletes one workflow through the shared application composition.
    pub fn delete_workflow(
        &self,
        request: DeleteWorkflowRequest,
    ) -> Result<DeleteWorkflowResponse, BackendError> {
        self.workflow.delete(request).map_err(BackendError::from)
    }
    /// Gets the draft snapshot through the shared application composition.
    pub fn get_workflow_draft(
        &self,
        request: GetDraftRequest,
    ) -> Result<GetDraftResponse, BackendError> {
        self.workflow.get_draft(request).map_err(BackendError::from)
    }
    /// Updates the draft snapshot through the shared application composition.
    pub fn update_workflow_draft(
        &self,
        request: UpdateDraftRequest,
    ) -> Result<UpdateDraftResponse, BackendError> {
        self.workflow
            .update_draft(request)
            .map_err(BackendError::from)
    }
    /// Publishes a workflow draft through the shared application composition.
    pub fn publish_workflow(
        &self,
        request: PublishWorkflowRequest,
    ) -> Result<PublishWorkflowResponse, BackendError> {
        self.workflow.publish(request).map_err(BackendError::from)
    }
    /// Rolls back the draft through the shared application composition.
    pub fn rollback_workflow(
        &self,
        request: RollbackWorkflowRequest,
    ) -> Result<RollbackWorkflowResponse, BackendError> {
        self.workflow.rollback(request).map_err(BackendError::from)
    }
    /// Activates a published version through the shared application composition.
    pub fn activate_workflow(
        &self,
        request: ActivateWorkflowRequest,
    ) -> Result<ActivateWorkflowResponse, BackendError> {
        self.workflow.activate(request).map_err(BackendError::from)
    }
    /// Lists published versions through the shared application composition.
    pub fn list_workflow_versions(
        &self,
        request: ListVersionsRequest,
    ) -> Result<ListVersionsResponse, BackendError> {
        self.workflow
            .list_versions(request)
            .map_err(BackendError::from)
    }
    /// Gets one version snapshot through the shared application composition.
    pub fn get_workflow_version(
        &self,
        request: GetVersionRequest,
    ) -> Result<GetVersionResponse, BackendError> {
        self.workflow
            .get_version(request)
            .map_err(BackendError::from)
    }
    /// Deletes one version snapshot through the shared application composition.
    pub fn delete_workflow_snapshot(
        &self,
        request: DeleteSnapshotRequest,
    ) -> Result<DeleteSnapshotResponse, BackendError> {
        self.workflow
            .delete_snapshot(request)
            .map_err(BackendError::from)
    }
    /// Gets one snapshot by its stable identifier through the shared application composition.
    pub fn get_workflow_snapshot(
        &self,
        request: GetWorkflowSnapshotRequest,
    ) -> Result<GetWorkflowSnapshotResponse, BackendError> {
        self.workflow
            .get_snapshot(request)
            .map_err(BackendError::from)
    }

    // =============================================================================
    // workflowRun
    // =============================================================================

    /// Creates one workflow run through the shared application composition.
    pub fn create_workflow_run(
        &self,
        request: CreateWorkflowRunRequest,
    ) -> Result<CreateWorkflowRunResponse, BackendError> {
        self.workflow_run
            .create(request)
            .map_err(BackendError::from)
    }
    /// Gets one workflow run through the shared application composition.
    pub fn get_workflow_run(
        &self,
        request: GetWorkflowRunRequest,
    ) -> Result<GetWorkflowRunResponse, BackendError> {
        self.workflow_run.get(request).map_err(BackendError::from)
    }
    /// Lists workflow runs for one project through the shared application composition.
    pub fn list_workflow_runs(
        &self,
        request: ListWorkflowRunsRequest,
    ) -> Result<ListWorkflowRunsResponse, BackendError> {
        self.workflow_run.list(request).map_err(BackendError::from)
    }
    /// Lists workflow runs for one workflow through the shared application composition.
    pub fn list_workflow_runs_by_workflow(
        &self,
        request: ListWorkflowRunsByWorkflowRequest,
    ) -> Result<ListWorkflowRunsByWorkflowResponse, BackendError> {
        self.workflow_run
            .list_by_workflow(request)
            .map_err(BackendError::from)
    }
    /// Lists the node-run history of one run through the shared application composition.
    pub fn list_workflow_node_runs(
        &self,
        request: ListWorkflowNodeRunsRequest,
    ) -> Result<ListWorkflowNodeRunsResponse, BackendError> {
        self.workflow_run
            .list_node_runs(request)
            .map_err(BackendError::from)
    }
    /// Deletes one workflow run through the shared application composition.
    pub fn delete_workflow_run(
        &self,
        request: DeleteWorkflowRunRequest,
    ) -> Result<DeleteWorkflowRunResponse, BackendError> {
        self.workflow_run
            .delete(request)
            .map_err(BackendError::from)
    }
}

/// Runs one blocking repository operation off the async runtime's worker threads.
///
/// The SQLite work behind a delete genuinely blocks: acquiring a pooled
/// connection waits when every slot is taken, and a cascading delete opens an
/// immediate transaction that parks on the busy timeout while another writer
/// holds the reservation. Parking an async worker for that long starves every
/// other request the runtime is serving, so the wait belongs on the blocking
/// pool even though the caller is asynchronous for unrelated reasons.
async fn spawn_repository_work<T>(
    work: impl FnOnce() -> Result<T, BackendError> + Send + 'static,
) -> Result<T, BackendError>
where
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|source| BackendError::internal("repository operation did not complete", source))?
}

/// Creates one required runtime directory and preserves its exact failing path.
fn ensure_directory(path: &Path) -> Result<(), BackendBootstrapError> {
    fs::create_dir_all(path).map_err(|source| BackendBootstrapError::DirectoryCreate {
        path: path.to_path_buf(),
        source,
    })
}

/// Fails runs interrupted by a previous process, keeping `current_nodes` intact.
///
/// Runs that were `Running` or `Failed` when the process died have their non-terminal node runs
/// marked `Failed` with `interrupted_by_restart`; the sweep is idempotent and best-effort so a
/// storage failure cannot block startup.
fn run_workflow_run_boot_sweep(pool: &RepositoryPool, clock: SystemClock) {
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let run_ids = match repository.list_recoverable_runs() {
        Ok(run_ids) => run_ids,
        Err(error) => {
            ora_error!(error = %error, "workflow run boot sweep failed to list recoverable runs");
            return;
        }
    };
    if run_ids.is_empty() {
        return;
    }
    if let Err(error) = repository.fail_orphaned_node_runs(&run_ids, clock.now_timestamp_millis()) {
        ora_error!(error = %error, "workflow run boot sweep failed to fail orphaned node runs");
    }
}

#[cfg(test)]
mod tests {
    use super::{Backend, BackendPaths};
    use crate::error::ErrorClassification;
    use ora_contracts::CreateTaskRequest;
    use ora_contracts::{
        CreateAgentRequest, CreateProjectRequest, CreateSkillRequest, DeleteAgentRequest,
        DeleteProjectRequest, DeleteSkillRequest, DeleteTaskRequest, GetProjectRequest,
        GetTaskRequest, ListAgentsRequest, ListProjectsRequest, ListSkillsRequest,
        UpdateAgentRequest, UpdateProjectRequest, UpdateSkillRequest,
    };
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use tempfile::TempDir;

    /// Verifies the shared composition owns storage bootstrap and complete non-Git CRUD flows.
    #[tokio::test]
    async fn opens_storage_and_serves_shared_crud_apis() {
        let temporary = TempDir::new().expect("create temporary backend directory");
        let database_path = temporary.path().join("data").join("ora.sqlite3");
        let worktree_root = temporary.path().join("worktrees");
        let backend = Backend::open(BackendPaths {
            database_path: database_path.clone(),
            data_directory: temporary.path().to_path_buf(),
            deno_path: std::path::PathBuf::from("deno"),
            worktree_root: worktree_root.clone(),
            home_directory: temporary.path().to_path_buf(),
            relative_path_base: temporary.path().to_path_buf(),
            sessions_root: temporary.path().join("sessions"),
            skills_root: temporary.path().join("atoms").join("skills"),
            ripgrep_path: std::path::PathBuf::from("rg"),
            timezone: chrono_tz::UTC,
        })
        .expect("open shared backend");

        assert!(database_path.is_file());
        assert!(worktree_root.is_dir());

        let project = backend
            .create_project(CreateProjectRequest {
                name: "Ora".to_string(),
                root_path: temporary
                    .path()
                    .join("repository")
                    .to_string_lossy()
                    .into_owned(),
            })
            .expect("create project")
            .project;
        let updated_project = backend
            .update_project(UpdateProjectRequest {
                project_id: project.id.clone(),
                name: "Ora Desktop".to_string(),
            })
            .expect("update project")
            .project;
        assert_eq!(updated_project.name, "Ora Desktop");
        assert_eq!(
            backend
                .list_projects(ListProjectsRequest {})
                .expect("list projects")
                .projects,
            vec![updated_project.clone()]
        );

        let skill = backend
            .create_skill(CreateSkillRequest {
                name: "review".to_string(),
                description: "Review changes".to_string(),
                content: None,
            })
            .expect("create skill")
            .skill;
        let skill = backend
            .update_skill(UpdateSkillRequest {
                skill_id: skill.id,
                name: "review-code".to_string(),
                description: "Review implementation changes".to_string(),
                content: None,
            })
            .expect("update skill")
            .skill;
        assert_eq!(
            backend
                .list_skills(ListSkillsRequest {})
                .expect("list skills")
                .skills,
            vec![skill.clone()]
        );

        let agent = backend
            .create_agent(CreateAgentRequest {
                name: "codex".to_string(),
                description: "Coding agent".to_string(),
                content: None,
            })
            .expect("create agent")
            .agent;
        let agent = backend
            .update_agent(UpdateAgentRequest {
                agent_id: agent.id,
                name: "codex-desktop".to_string(),
                description: "Desktop coding agent".to_string(),
                content: None,
            })
            .expect("update agent")
            .agent;
        assert_eq!(
            backend
                .list_agents(ListAgentsRequest {})
                .expect("list agents")
                .agents,
            vec![agent.clone()]
        );

        backend
            .delete_agent(DeleteAgentRequest { agent_id: agent.id })
            .expect("delete agent");
        backend
            .delete_skill(DeleteSkillRequest { skill_id: skill.id })
            .expect("delete skill");
        backend
            .delete_project(DeleteProjectRequest {
                project_id: project.id.clone(),
            })
            .await
            .expect("delete project");

        let error = backend
            .get_project(GetProjectRequest {
                project_id: project.id,
            })
            .expect_err("deleted project should be hidden");
        assert_eq!(error.classification(), ErrorClassification::NotFound);
        assert_eq!(error.public_error().code(), "project_not_found");
    }

    /// Verifies an update rewrites only the manifest and preserves other package files.
    #[test]
    fn update_preserves_other_package_files() {
        let temporary = TempDir::new().expect("create temporary backend directory");
        let skills_root = temporary.path().join("atoms").join("skills");
        let backend = Backend::open(BackendPaths {
            database_path: temporary.path().join("ora.sqlite3"),
            data_directory: temporary.path().to_path_buf(),
            deno_path: std::path::PathBuf::from("deno"),
            worktree_root: temporary.path().join("worktrees"),
            home_directory: temporary.path().to_path_buf(),
            relative_path_base: temporary.path().to_path_buf(),
            sessions_root: temporary.path().join("sessions"),
            skills_root: skills_root.clone(),
            ripgrep_path: std::path::PathBuf::from("rg"),
            timezone: chrono_tz::UTC,
        })
        .expect("open shared backend");

        let skill = backend
            .create_skill(CreateSkillRequest {
                name: "review".to_string(),
                description: "Reviews changes".to_string(),
                content: None,
            })
            .expect("create skill")
            .skill;
        // A user-added package file must survive an ordinary update.
        fs::create_dir_all(skills_root.join("review")).expect("create package directory");
        fs::write(skills_root.join("review").join("helper.sh"), "echo hi")
            .expect("write helper file");

        let updated = backend
            .update_skill(UpdateSkillRequest {
                skill_id: skill.id,
                name: "review".to_string(),
                description: "Reviews pull requests".to_string(),
                content: None,
            })
            .expect("update skill")
            .skill;
        assert_eq!(updated.description, "Reviews pull requests");
        assert!(skills_root.join("review").join("helper.sh").is_file());
        let manifest =
            fs::read_to_string(skills_root.join("review").join("SKILL.md")).expect("read manifest");
        assert!(manifest.contains("description: Reviews pull requests"));
    }

    /// Verifies task deletion hides Ora records while deliberately preserving the Git worktree.
    #[tokio::test]
    async fn deletes_existing_task_after_worktree_root_changes() {
        let temporary = TempDir::new().expect("create temporary backend directory");
        let repository_root = temporary.path().join("repository");
        initialize_repository(&repository_root);
        let original_worktree_root = temporary.path().join("original-worktrees");
        let backend = Backend::open(BackendPaths {
            database_path: temporary.path().join("ora.sqlite3"),
            data_directory: temporary.path().to_path_buf(),
            deno_path: std::path::PathBuf::from("deno"),
            worktree_root: original_worktree_root.clone(),
            home_directory: temporary.path().to_path_buf(),
            relative_path_base: temporary.path().to_path_buf(),
            sessions_root: temporary.path().join("sessions"),
            skills_root: temporary.path().join("atoms").join("skills"),
            ripgrep_path: std::path::PathBuf::from("rg"),
            timezone: chrono_tz::UTC,
        })
        .expect("open shared backend");
        let project = backend
            .create_project(CreateProjectRequest {
                name: "Ora".to_string(),
                root_path: repository_root.to_string_lossy().into_owned(),
            })
            .expect("create project")
            .project;
        let task = backend
            .create_task(CreateTaskRequest {
                project_id: project.id,
                title: "Move configuration".to_string(),
                workspace_mode: None,
                base_branch: Some("main".to_string()),
            })
            .expect("create task")
            .task;
        let original_worktree_path = original_worktree_root.join(&task.id);
        assert!(original_worktree_path.is_dir());

        let replacement_root = temporary.path().join("replacement-worktrees");
        fs::create_dir_all(&replacement_root).expect("create replacement worktree root");
        backend
            .set_worktree_root(replacement_root)
            .expect("replace worktree creation root");
        backend
            .delete_task(DeleteTaskRequest {
                task_id: task.id.clone(),
            })
            .await
            .expect("delete task without Git mutation");

        assert!(original_worktree_path.exists());
        assert!(
            backend
                .get_task(GetTaskRequest { task_id: task.id })
                .is_err()
        );
    }

    /// Initializes a repository with one commit so linked worktree operations are available.
    fn initialize_repository(repository_root: &Path) {
        fs::create_dir_all(repository_root).expect("create repository root");
        run_git(repository_root, &["init", "--initial-branch=main"]);
        run_git(repository_root, &["config", "user.name", "Ora Tests"]);
        run_git(
            repository_root,
            &["config", "user.email", "ora-tests@example.com"],
        );
        fs::write(repository_root.join("README.md"), "ora backend test\n")
            .expect("write repository seed file");
        run_git(repository_root, &["add", "README.md"]);
        run_git(repository_root, &["commit", "-m", "initial"]);
    }

    /// Runs a required Git setup command and preserves its exact arguments in failures.
    fn run_git(repository_root: &Path, arguments: &[&str]) {
        let status = Command::new("git")
            .current_dir(repository_root)
            .args(arguments)
            .status()
            .unwrap_or_else(|error| panic!("failed to start git {arguments:?}: {error}"));

        assert!(status.success(), "git {arguments:?} failed with {status}");
    }
}
