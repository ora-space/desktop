use crate::agent::AgentApi;
use crate::agent_runtime::{
    AgentRuntimeManager, AgentRuntimeSetup, SessionEventStream, SessionLocator,
};
use crate::app_event::AppEventHub;
use crate::clock::SystemClock;
use crate::error::{BackendError, ErrorClassification};
use crate::git_cleanup::KeyedResourceLocks;
use crate::plugin::PluginApi;
use crate::plugin_gateway::PluginGateway;
use crate::project::ProjectApi;
use crate::session::SessionApi;
use crate::skill::SkillApi;
use crate::spec::SpecApi;
use crate::task::TaskApi;
use crate::task_diff::TaskDiffApi;
use crate::user_config::{BackendPreferredLogLevelStore, UserConfigApi};
use crate::workflow::WorkflowApi;
use crate::workflow::run::WorkflowRunApi;
use crate::workflow::run::{
    ConcreteWorkflowRunControl, ConcreteWorkflowRunEngine, build_workflow_run_engine,
};
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
    /// Directory against which persisted relative local Workspace locations are resolved.
    ///
    /// Relative locations are stored against the directory from which `ORA_DATA_DIR`
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
    #[error("failed to initialize plugin management")]
    Plugin(#[source] BackendError),
    #[error("failed to synchronize installed plugin Skills")]
    PluginSkillCatalog(#[source] BackendError),
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
    user_config: Arc<UserConfigApi>,
    session: Arc<SessionApi>,
    agent_runtime: Arc<AgentRuntimeManager>,
    plugin: Arc<PluginApi>,
    skill: Arc<SkillApi>,
    agent: Arc<AgentApi>,
    spec: Arc<SpecApi>,
    workflow: Arc<WorkflowApi>,
    workflow_run: Arc<WorkflowRunApi>,
    workflow_run_engine: Arc<ConcreteWorkflowRunControl>,
    /// Serializes scheduling-affecting workflow-run mutations per run across the control entry
    /// points, the manual completion path, and the session-driver callback.
    run_locks: Arc<KeyedResourceLocks>,
    /// Transient set of node runs a manual completion is currently claiming; blocks a concurrent
    /// prompt against the same node without adding any persisted status.
    completing_node_runs: Arc<crate::workflow::run::interactive::CompletingNodeRuns>,
    sessions_root: PathBuf,
    baselines_root: PathBuf,
    app_events: Arc<AppEventHub>,
    git_cleanup: crate::git_cleanup::GitCleanupHandle,
    relative_path_base: PathBuf,
}

impl Backend {
    /// Opens persistent storage and constructs every shared CRUD API.
    ///
    /// Installed agent plugins join the built-in CLIs as agent providers; they are discovered by
    /// the plugin lifecycle under `paths.data_directory`, which also owns their processes.
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
        let user_config = Arc::new(UserConfigApi::new(pool.clone()));
        let plugin = Arc::new(
            PluginApi::open(
                pool.clone(),
                paths.data_directory,
                paths.deno_path,
                clock,
                app_events.publisher(),
                user_config.clone(),
            )
            .map_err(BackendBootstrapError::Plugin)?,
        );
        plugin
            .sync_installed_skills()
            .map_err(BackendBootstrapError::PluginSkillCatalog)?;
        let scheduler = Scheduler::new(paths.timezone);
        let worktree_root = Arc::new(RwLock::new(paths.worktree_root));
        let sessions_root = paths.sessions_root;
        // Side files holding the worktree baseline an interactive node diffs at completion.
        let baselines_root = sessions_root.join("node-baselines");
        let relative_path_base = paths.relative_path_base;
        let agent_runtime = Arc::new(
            AgentRuntimeManager::new(AgentRuntimeSetup {
                plugin_host: plugin.clone(),
                pool: pool.clone(),
                home_directory: paths.home_directory,
                relative_path_base: relative_path_base.clone(),
                sessions_root: sessions_root.clone(),
                clock,
                scheduler,
                app_events: app_events.publisher(),
            })
            .map_err(BackendBootstrapError::AgentRuntime)?,
        );
        // Build the run engine before the crash sweep so recovery can resume stalled runs.
        let workflow_run_assembly = build_workflow_run_engine(
            agent_runtime.clone(),
            pool.clone(),
            baselines_root.clone(),
            clock,
        );
        let workflow_run_engine = workflow_run_assembly.control;
        let run_locks = workflow_run_assembly.run_locks;
        let workflow_engine = workflow_run_assembly.engine;

        // Crash recovery: fail orphaned node runs, then reconcile stalled Running runs left by a
        // previous process before serving new commands (best-effort; a failure must not block
        // startup).
        run_workflow_run_boot_sweep(&pool, &workflow_engine, &run_locks, clock);
        // Reclaim orphaned worktree-baseline side files left by a previous process.
        prune_orphaned_baselines(&pool, &baselines_root);

        // Durable Git cleanup: the worker's first pass replays every cleanup job
        // and expired provisioning lease a previous process left behind.
        let git_cleanup_worker =
            crate::git_cleanup::GitCleanupWorker::new(pool.clone(), worktree_root.clone(), clock);
        let repository_gates = git_cleanup_worker.repository_gates();
        let git_cleanup = git_cleanup_worker.spawn();

        // Durable Effect reconciliation: the first pass replays every surface a previous process
        // left short of its Desired generation, including the retirement cleanup an uninstall
        // started but could not finish.
        let effect_worker = crate::effect_worker::EffectWorker::new(pool.clone(), plugin.clone());
        effect_worker.recover();
        plugin.set_effect_reconcile(effect_worker.spawn());

        Ok(Self {
            project: Arc::new(ProjectApi::new(pool.clone(), sessions_root.clone(), clock)),
            task: Arc::new(TaskApi::new(
                pool.clone(),
                worktree_root.clone(),
                relative_path_base.clone(),
                sessions_root.clone(),
                repository_gates,
                clock,
            )),
            task_diff: Arc::new(TaskDiffApi::new(
                pool.clone(),
                git_cleanup.clone(),
                relative_path_base.clone(),
            )),
            user_config,
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
            workflow_run: Arc::new(WorkflowRunApi::new(pool.clone(), paths.skills_root, clock)),
            workflow_run_engine,
            run_locks,
            completing_node_runs: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            sessions_root,
            baselines_root,
            app_events,
            git_cleanup,
            pool,
            worktree_root,
            relative_path_base,
        })
    }

    /// Returns the plugin data-plane gateway the desktop surface layer drives.
    pub fn plugin_gateway(&self) -> Arc<PluginGateway> {
        Arc::new(PluginGateway::new(Arc::clone(&self.plugin)))
    }

    /// Returns the cached installed-plugin snapshot without rescanning the filesystem.
    pub fn list_installed_plugins(
        &self,
        request: ListInstalledPluginsRequest,
    ) -> Result<ListInstalledPluginsResponse, BackendError> {
        Ok(self.plugin.list(request))
    }

    /// Returns one typed Plugin Configuration editor snapshot.
    pub fn get_plugin_configuration(
        &self,
        request: GetPluginConfigurationRequest,
    ) -> Result<GetPluginConfigurationResponse, BackendError> {
        self.plugin.get_configuration(request)
    }

    /// Persists one revision-checked Plugin Configuration replacement.
    pub fn save_plugin_configuration(
        &self,
        request: SavePluginConfigurationRequest,
    ) -> Result<SavePluginConfigurationResponse, BackendError> {
        self.plugin.save_configuration(request)
    }

    /// Executes an explicit Reset All or damaged-data recovery operation.
    pub fn reset_plugin_configuration(
        &self,
        request: ResetPluginConfigurationRequest,
    ) -> Result<ResetPluginConfigurationResponse, BackendError> {
        self.plugin.reset_configuration(request)
    }

    /// Returns the cached marketplace registry index used to populate plugin discovery.
    pub fn list_available_plugins(
        &self,
        request: ListAvailablePluginsRequest,
    ) -> Result<ListAvailablePluginsResponse, BackendError> {
        self.plugin
            .list_available_plugins(request)
            .map_err(|error| BackendError::internal("failed to load plugin registry index", error))
    }

    /// Returns every configured marketplace source in precedence order.
    pub fn list_marketplace_sources(
        &self,
        request: ListMarketplaceSourcesRequest,
    ) -> Result<ListMarketplaceSourcesResponse, BackendError> {
        self.plugin.list_marketplace_sources(request)
    }

    /// Adds one marketplace source after validating and persisting it.
    pub fn add_marketplace_source(
        &self,
        request: AddMarketplaceSourceRequest,
    ) -> Result<AddMarketplaceSourceResponse, BackendError> {
        self.plugin.add_marketplace_source(request)
    }

    /// Removes one marketplace source by URL after persisting the new ordering.
    pub fn delete_marketplace_source(
        &self,
        request: DeleteMarketplaceSourceRequest,
    ) -> Result<DeleteMarketplaceSourceResponse, BackendError> {
        self.plugin.delete_marketplace_source(request)
    }

    /// Changes one marketplace source\u2019s proxy policy after persisting it.
    pub fn update_marketplace_source(
        &self,
        request: UpdateMarketplaceSourceRequest,
    ) -> Result<UpdateMarketplaceSourceResponse, BackendError> {
        self.plugin.update_marketplace_source(request)
    }

    /// Pulls the marketplace source and rebuilds the cache used by plugin discovery.
    pub fn sync_available_plugins(
        &self,
        request: SyncAvailablePluginsRequest,
    ) -> Result<SyncAvailablePluginsResponse, BackendError> {
        self.plugin.sync_available_plugins(request)
    }

    /// Explicitly rescans packages and reconciles process-local runtime state.
    pub async fn scan_plugins(
        &self,
        request: ScanPluginsRequest,
    ) -> Result<ScanPluginsResponse, BackendError> {
        let response = self
            .plugin
            .scan(request)
            .await
            .map_err(BackendError::from)?;
        self.agent_runtime.sync_plugin_agents();
        Ok(response)
    }

    /// Starts one installed plugin and returns its immediate starting state.
    pub async fn activate_plugin(
        &self,
        request: ActivatePluginRequest,
    ) -> Result<ActivatePluginResponse, BackendError> {
        self.plugin
            .activate(request)
            .await
            .map_err(BackendError::from)
    }

    /// Stops one plugin process while leaving the installed plugin available.
    pub async fn stop_plugin(
        &self,
        request: StopPluginRequest,
    ) -> Result<StopPluginResponse, BackendError> {
        self.plugin.stop(request).await.map_err(BackendError::from)
    }

    /// Stops and removes one plugin package plus its process-local state.
    pub async fn uninstall_plugin(
        &self,
        request: UninstallPluginRequest,
    ) -> Result<UninstallPluginResponse, BackendError> {
        let response = self.plugin.uninstall(request).await?;
        self.agent_runtime.sync_plugin_agents();
        Ok(response)
    }

    /// Installs a marketplace plugin by resolving its release manifest from the synced source and
    /// downloading, verifying, and extracting its package through the network-backed installer.
    ///
    /// The agent set is reconciled afterwards so the newly installed package supplies a reachable
    /// agent in this process rather than only after the next restart.
    pub async fn install_plugin(
        &self,
        request: InstallPluginRequest,
    ) -> Result<InstallPluginResponse, BackendError> {
        let response = self.plugin.install(request).await?;
        self.agent_runtime.sync_plugin_agents();
        Ok(response)
    }

    /// Imports one local release archive and reconciles the agent set afterwards.
    ///
    /// The agent set is reconciled so the imported package supplies a reachable agent in this
    /// process rather than only after the next restart.
    pub async fn import_plugin(
        &self,
        request: ImportPluginRequest,
    ) -> Result<ImportPluginResponse, BackendError> {
        let response = self.plugin.import(request).await?;
        self.agent_runtime.sync_plugin_agents();
        Ok(response)
    }

    /// Starts a workflow run against its frozen snapshot graph.
    pub fn start_workflow_run(
        &self,
        request: StartWorkflowRunRequest,
    ) -> Result<StartWorkflowRunResponse, BackendError> {
        let _gate = self.run_locks.acquire_exclusive(request.run_id.clone());
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
    pub async fn complete_workflow_node(
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
        let engine = self.workflow_run_engine.clone();
        let run_locks = self.run_locks.clone();
        let completing = self.completing_node_runs.clone();
        let node_run_id = prepared.node_run_id.clone();
        let output = prepared.output.clone();
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
                    .complete_node(&run_id, &node_run_id, output, stop_reason, file_changes)
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
    pub fn restart_workflow_run(
        &self,
        request: RestartWorkflowRunRequest,
    ) -> Result<RestartWorkflowRunResponse, BackendError> {
        let _gate = self.run_locks.acquire_exclusive(request.run_id.clone());
        self.workflow_run_engine
            .restart(request)
            .map_err(BackendError::from)
    }

    /// Sets the kickoff input of a pending workflow run.
    pub fn update_workflow_run_input(
        &self,
        request: UpdateWorkflowRunInputRequest,
    ) -> Result<UpdateWorkflowRunInputResponse, BackendError> {
        let _gate = self.run_locks.acquire_exclusive(request.run_id.clone());
        self.workflow_run_engine
            .update_input(request)
            .map_err(BackendError::from)
    }

    /// Returns the repository pool needed by server-only services excluded from this extraction.
    pub fn repository_pool(&self) -> RepositoryPool {
        self.pool.clone()
    }

    /// Returns the authoritative shared developer-mode preference.
    pub async fn developer_mode(&self) -> Result<ora_application::DeveloperMode, BackendError> {
        self.user_config.developer_mode().await
    }

    /// Persists and returns the authoritative shared developer-mode preference.
    pub async fn set_developer_mode(
        &self,
        mode: ora_application::DeveloperMode,
    ) -> Result<ora_application::DeveloperMode, BackendError> {
        self.user_config.set_developer_mode(mode).await
    }

    /// Returns the preferred runtime log level stored in shared user configuration.
    pub async fn preferred_log_level(&self) -> Result<ora_logging::LogLevel, BackendError> {
        self.user_config.preferred_log_level().await
    }

    /// Persists and returns the preferred runtime log level in shared user configuration.
    pub async fn set_preferred_log_level(
        &self,
        level: ora_logging::LogLevel,
    ) -> Result<ora_logging::LogLevel, BackendError> {
        self.user_config.set_preferred_log_level(level).await
    }

    /// Returns the optional configured network proxy settings.
    pub fn network_proxy_settings(
        &self,
    ) -> Result<Option<ora_application::NetworkProxySettings>, BackendError> {
        self.user_config.network_proxy_settings()
    }

    /// Persists and returns the configured network proxy settings.
    pub fn set_network_proxy_settings(
        &self,
        settings: ora_application::NetworkProxySettings,
    ) -> Result<ora_application::NetworkProxySettings, BackendError> {
        self.user_config.set_network_proxy_settings(settings)
    }

    /// Returns the restricted preferred-level persistence capability for runtime logging.
    pub fn preferred_log_level_store(&self) -> BackendPreferredLogLevelStore {
        BackendPreferredLogLevelStore::new(self.user_config.clone())
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

    /// Resolves the project checkout root used before a task exists (draft / warm chat).
    ///
    /// Resolves the local directory backing one Workspace for host integrations.
    pub fn resolve_workspace_cwd(&self, workspace_id: &str) -> Result<PathBuf, BackendError> {
        crate::task::resolve_workspace_cwd(
            &self.pool,
            &ora_domain::WorkspaceId::new(workspace_id),
            &self.relative_path_base,
        )
    }

    /// Resolves the main Workspace directory for an ordinary project chat.
    pub fn resolve_project_cwd(&self, project_id: &str) -> Result<PathBuf, BackendError> {
        crate::task::resolve_project_cwd(
            &self.pool,
            &ora_domain::ProjectId::new(project_id),
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

    /// Lists visible workspaces directly from their Workspace-owned persistence boundary.
    pub fn list_workspaces(
        &self,
        _request: ListWorkspacesRequest,
    ) -> Result<ListWorkspacesResponse, BackendError> {
        let workspaces = ora_db::SqliteWorkspaceRepository::new(self.pool.clone())
            .list_all_workspaces()
            .map_err(|error| BackendError::internal("failed to list workspaces", error))?;
        Ok(ListWorkspacesResponse {
            workspaces: workspaces
                .into_iter()
                .map(|workspace| Workspace {
                    id: workspace.id.to_string(),
                    project_id: workspace.project_id.to_string(),
                    kind: match workspace.kind {
                        ora_domain::WorkspaceKind::Main => WorkspaceKind::Main,
                        ora_domain::WorkspaceKind::Isolated => WorkspaceKind::Isolated,
                    },
                    lifecycle: match workspace.lifecycle {
                        ora_domain::WorkspaceLifecycle::Provisioning => {
                            WorkspaceLifecycle::Provisioning
                        }
                        ora_domain::WorkspaceLifecycle::Active => WorkspaceLifecycle::Active,
                        ora_domain::WorkspaceLifecycle::Unavailable => {
                            WorkspaceLifecycle::Unavailable
                        }
                        ora_domain::WorkspaceLifecycle::Retiring => WorkspaceLifecycle::Retiring,
                        ora_domain::WorkspaceLifecycle::Deleted => WorkspaceLifecycle::Deleted,
                    },
                })
                .collect(),
        })
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
    /// The delete cascades to the project's Workspaces and Tasks, so every chat
    /// surface beneath it dies along with their warm sessions and histories are
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
        let (response, workspace_ids) = spawn_repository_work(move || {
            let workspace_ids = crate::task::workspace_ids_in_project(&pool, &project_id);
            project
                .delete(request)
                .map(|response| (response, workspace_ids))
        })
        .await?;

        let targets: Vec<WarmSessionTarget> = workspace_ids
            .into_iter()
            .map(|workspace_id| WarmSessionTarget::Workspace {
                workspace_id: workspace_id.to_string(),
            })
            .collect();
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
            .discard_warm_sessions(&[WarmSessionTarget::Workspace {
                workspace_id: response.workspace_id.clone(),
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
        // Snapshot unpublished ownership before reading SQLite. If a node binding commits between
        // these reads, this snapshot still excludes the row returned by the earlier database view;
        // a later request instead sees the committed binding through the repository filter.
        let unpublished = self.agent_runtime.unpublished_workflow_session_ids()?;
        let mut response = self.session.list(request).map_err(BackendError::from)?;
        response
            .sessions
            .retain(|session| !unpublished.contains(&session.id));
        Ok(response)
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
    /// Loads one session conversation and continues its active turn when present.
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
    ///
    /// When the session belongs to an awaiting interactive workflow node, the node flips to
    /// `Running` for the duration of the turn and back to `Pending` when the turn ends or the
    /// stream is dropped, so the node's awaiting status tracks the agent's generating state.
    pub async fn prompt_session(
        &self,
        request: PromptSessionRequest,
    ) -> Result<SessionEventStream<PromptSessionEvent>, BackendError> {
        let node_run_id = crate::workflow::run::interactive::begin_human_turn(
            &self.pool,
            &self.run_locks,
            &self.completing_node_runs,
            &request.session_id,
        )
        .await?;
        let stream = match self.agent_runtime.prompt_session(request).await {
            Ok(stream) => stream,
            Err(error) => {
                // The turn never started; put the awaiting node back where it was.
                if let Some(node_run_id) = node_run_id.as_ref() {
                    let _ =
                        crate::workflow::run::interactive::end_human_turn(&self.pool, node_run_id)
                            .await;
                }
                return Err(error);
            }
        };
        let Some(node_run_id) = node_run_id else {
            return Ok(stream);
        };
        let pool = self.pool.clone();
        Ok(stream.attach_cleanup(move || {
            tokio::spawn(async move {
                let _ =
                    crate::workflow::run::interactive::end_human_turn(&pool, &node_run_id).await;
            });
        }))
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

    /// Cancels one active prompt while keeping its session available for another turn.
    pub fn cancel_session_prompt(
        &self,
        request: CancelSessionPromptRequest,
    ) -> Result<CancelSessionPromptResponse, BackendError> {
        self.agent_runtime.cancel_session_prompt(request)
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

    /// Lists the models one agent advertises outside any session.
    pub fn list_agent_models(
        &self,
        request: ListAgentModelsRequest,
    ) -> Result<ListAgentModelsResponse, BackendError> {
        self.agent_runtime.agent_models(request)
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

    /// Renames one workflow run through its Workspace-owned display field.
    pub fn rename_workflow_run(
        &self,
        request: RenameWorkflowRunRequest,
    ) -> Result<RenameWorkflowRunResponse, BackendError> {
        self.workflow_run
            .rename(request)
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
pub(crate) async fn spawn_repository_work<T>(
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

/// Fails runs interrupted by a previous process, then reconciles the survivors.
///
/// Runs that were `Running` or `Failed` when the process died have their non-terminal node runs
/// marked `Failed` with `interrupted_by_restart`. Surviving `Running` runs are then reconciled:
/// stalled ones resume scheduling, and invalid `Pending` nodes fail closed. The sweep is
/// idempotent and best-effort so a storage failure cannot block startup.
fn run_workflow_run_boot_sweep(
    pool: &RepositoryPool,
    engine: &Arc<ConcreteWorkflowRunEngine>,
    run_locks: &Arc<KeyedResourceLocks>,
    clock: SystemClock,
) {
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let run_ids = match repository.list_recoverable_runs() {
        Ok(run_ids) => run_ids,
        Err(error) => {
            ora_error!(error = %error, "workflow run boot sweep failed to list recoverable runs");
            return;
        }
    };
    if !run_ids.is_empty()
        && let Err(error) =
            repository.fail_orphaned_node_runs(&run_ids, clock.now_timestamp_millis())
    {
        ora_error!(error = %error, "workflow run boot sweep failed to fail orphaned node runs");
    }
    crate::workflow::run::reconcile_running_workflow_runs(engine, run_locks, pool);
}

/// Deletes worktree-baseline side files whose node run is missing or no longer awaiting input.
///
/// Baselines exist only while an interactive node awaits input; a crash between a node's terminal
/// commit and its baseline deletion, or a node that failed without cleanup, leaves orphaned side
/// files that this sweep reclaims at the next boot.
fn prune_orphaned_baselines(pool: &RepositoryPool, baselines_root: &Path) {
    let Ok(entries) = std::fs::read_dir(baselines_root) else {
        return;
    };
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let node_run_id = ora_domain::WorkflowNodeRunId::new(name);
        let still_awaiting = repository
            .find_node_run_by_id(&node_run_id)
            .map(|node_run| {
                node_run.is_some_and(|node| node.status == ora_domain::WorkflowNodeStatus::Pending)
            })
            .unwrap_or(false);
        if !still_awaiting {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Backend, BackendPaths};
    use crate::error::ErrorClassification;
    use ora_application::DeveloperMode;
    use ora_contracts::CreateTaskRequest;
    use ora_contracts::{
        CreateAgentRequest, CreateProjectRequest, CreateSkillRequest, DeleteAgentRequest,
        DeleteProjectRequest, DeleteSkillRequest, DeleteTaskRequest, GetProjectRequest,
        GetTaskRequest, ListAgentsRequest, ListProjectsRequest, ListSkillsRequest,
        UpdateAgentRequest, UpdateProjectRequest, UpdateSkillRequest,
    };
    use ora_logging::LogLevel;
    use ora_test_support::GitTestScaffold;
    use std::fs;
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
        assert_eq!(
            (
                backend.developer_mode().await.unwrap(),
                backend.preferred_log_level().await.unwrap(),
            ),
            (DeveloperMode::Disabled, LogLevel::Info)
        );
        assert_eq!(
            (
                backend
                    .set_developer_mode(DeveloperMode::Enabled)
                    .await
                    .unwrap(),
                backend
                    .set_preferred_log_level(LogLevel::Debug)
                    .await
                    .unwrap(),
            ),
            (DeveloperMode::Enabled, LogLevel::Debug)
        );
        assert_eq!(
            (
                backend.developer_mode().await.unwrap(),
                backend.preferred_log_level().await.unwrap(),
            ),
            (DeveloperMode::Enabled, LogLevel::Debug)
        );

        let project = backend
            .create_project(CreateProjectRequest {
                name: "Ora".to_string(),
                main_workspace_path: temporary
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

    /// Verifies startup projects installed Skill plugins into the shared Skill catalog.
    #[test]
    fn opens_with_plugin_skills_written_to_the_existing_database_schema() {
        let temporary = TempDir::new().expect("create temporary backend directory");
        let package_root = temporary
            .path()
            .join("plugins/installed/official/review-pack/1.0.0");
        let skill_root = package_root.join("assets/review");
        fs::create_dir_all(&skill_root).expect("create installed Skill tree");
        fs::write(
            package_root.join("orax.toml"),
            "identifier = \"review-pack\"\nnamespace = \"official\"\nkind = \"skill\"\nversion = \"1.0.0\"\ndescription = \"Review skills\"\n",
        )
        .expect("write plugin manifest");
        fs::write(
            skill_root.join("SKILL.md"),
            "---\nname: review\ndescription: Reviews changes\n---\n# Review instructions\n",
        )
        .expect("write Skill manifest");

        let backend = Backend::open(BackendPaths {
            database_path: temporary.path().join("ora.sqlite3"),
            data_directory: temporary.path().to_path_buf(),
            deno_path: std::path::PathBuf::from("deno"),
            worktree_root: temporary.path().join("worktrees"),
            home_directory: temporary.path().to_path_buf(),
            relative_path_base: temporary.path().to_path_buf(),
            sessions_root: temporary.path().join("sessions"),
            skills_root: temporary.path().join("atoms/skills"),
            ripgrep_path: std::path::PathBuf::from("rg"),
            timezone: chrono_tz::UTC,
        })
        .expect("open shared backend");

        let skills = backend
            .list_skills(ListSkillsRequest {})
            .expect("list plugin Skills")
            .skills;
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].namespace, "official/review-pack");
        assert_eq!(skills[0].name, "review");
        assert_eq!(
            skills[0].source,
            ora_contracts::SkillSource::Plugin {
                plugin_id: "official/review-pack".to_string(),
            }
        );
        assert_eq!(
            skills[0].availability,
            ora_contracts::SkillAvailability::Available
        );
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
        let scaffold =
            GitTestScaffold::new("backend-task-deletion").expect("create Git test scaffold");
        scaffold
            .write_file(scaffold.repo_path(), "README.md", "ora backend test\n")
            .expect("write repository seed file");
        scaffold
            .stage_all_and_commit("initial")
            .expect("create repository seed commit");
        let repository_root = scaffold.repo_path().to_path_buf();
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
                main_workspace_path: repository_root.to_string_lossy().into_owned(),
            })
            .expect("create project")
            .project;
        let task = backend
            .create_task(CreateTaskRequest {
                project_id: project.id,
                title: "Move configuration".to_string(),
                base_branch: Some("main".to_string()),
            })
            .expect("create task")
            .task;
        let original_worktree_path = original_worktree_root.join(&task.workspace_id);
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

    /// Verifies a local Tavily MCP `.orax` import, configuration editor snapshot, and `store.json`
    /// persistence for the `apiKey` setting.
    #[tokio::test]
    async fn tavily_mcp_local_import_and_configuration() {
        use ora_contracts::{
            GetPluginConfigurationRequest, ImportPluginRequest, InstalledPluginContribution,
            ListInstalledPluginsRequest, PluginConfigurationCompleteness,
            PluginConfigurationSummary, PluginSettingValue, SavePluginConfigurationRequest,
        };
        use pretty_assertions::assert_eq;
        use std::collections::BTreeMap;
        use std::fs;
        use std::path::PathBuf;

        const PLUGIN_ID: &str = "official/ora-space.tavily-search";
        const TEST_API_KEY: &str = "tvly-test-e2e-key";

        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../");
        let orax_archive = workspace_root.join(".tmp/ora-space.tavily-search-v0.1.0.orax");
        if !orax_archive.is_file() {
            eprintln!(
                "skipping Tavily MCP import E2E: missing {}",
                orax_archive.display()
            );
            return;
        }

        let temporary = TempDir::new().expect("create temporary backend directory");
        let data_directory = temporary.path().to_path_buf();
        let backend = Backend::open(BackendPaths {
            database_path: data_directory.join("ora.sqlite3"),
            data_directory: data_directory.clone(),
            deno_path: std::path::PathBuf::from("deno"),
            worktree_root: data_directory.join("worktrees"),
            home_directory: data_directory.clone(),
            relative_path_base: data_directory.clone(),
            sessions_root: data_directory.join("sessions"),
            skills_root: data_directory.join("atoms").join("skills"),
            ripgrep_path: std::path::PathBuf::from("rg"),
            timezone: chrono_tz::UTC,
        })
        .expect("open shared backend");

        backend
            .import_plugin(ImportPluginRequest {
                path: orax_archive.to_string_lossy().into_owned(),
            })
            .await
            .expect("import Tavily MCP release");

        let installed = backend
            .list_installed_plugins(ListInstalledPluginsRequest {})
            .expect("list installed plugins")
            .plugins
            .into_iter()
            .find(|plugin| plugin.id == PLUGIN_ID)
            .expect("Tavily MCP plugin is installed");
        assert_eq!(
            installed.contribution,
            InstalledPluginContribution::Mcp,
            "installed plugin contribution"
        );
        assert_eq!(
            installed.configuration,
            PluginConfigurationSummary::Available {
                completeness: PluginConfigurationCompleteness::Incomplete,
            },
            "configuration is incomplete before apiKey is saved"
        );

        let configuration = backend
            .get_plugin_configuration(GetPluginConfigurationRequest {
                plugin_id: PLUGIN_ID.to_string(),
            })
            .expect("load plugin configuration editor")
            .configuration;
        let api_key_setting = configuration
            .settings
            .iter()
            .find(|setting| setting.declaration.id == "apiKey")
            .expect("apiKey setting is declared");
        assert_eq!(api_key_setting.declaration.title, "API key");

        let saved = backend
            .save_plugin_configuration(SavePluginConfigurationRequest {
                plugin_id: PLUGIN_ID.to_string(),
                expected_revision: configuration.revision,
                declaration_fingerprint: configuration.declaration_fingerprint.clone(),
                values: BTreeMap::from([(
                    "apiKey".to_string(),
                    PluginSettingValue::String(TEST_API_KEY.to_string()),
                )]),
            })
            .expect("save Tavily apiKey setting")
            .configuration;
        assert_eq!(
            saved.summary,
            PluginConfigurationSummary::Available {
                completeness: PluginConfigurationCompleteness::Complete,
            }
        );

        let store_json = fs::read_to_string(
            data_directory.join("plugins/data/official/ora-space.tavily-search/store.json"),
        )
        .expect("read persisted store.json");
        assert!(
            store_json.contains(TEST_API_KEY),
            "store.json should contain the saved apiKey value"
        );
    }

    /// Verifies marketplace registry resolution for the Tavily MCP listing without downloading
    /// the release archive.
    #[test]
    fn tavily_mcp_marketplace_manifest_resolves_from_staged_registry() {
        use ora_domain::PluginId;
        use ora_plugin_registry::RegistryIndex;
        use pretty_assertions::assert_eq;
        use std::fs;
        use std::path::PathBuf;

        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../");
        let marketplace_registry = workspace_root.join(".tmp/marketplace/registry");
        if !marketplace_registry.is_dir() {
            eprintln!(
                "skipping Tavily MCP marketplace manifest E2E: missing {}",
                marketplace_registry.display()
            );
            return;
        }

        let temporary = TempDir::new().expect("create temporary backend directory");
        let marketplace_checkout = temporary
            .path()
            .join("plugins/sources/github.com/ora-space/marketplace");
        fs::create_dir_all(&marketplace_checkout).expect("create marketplace checkout");
        copy_dir_recursive(
            &marketplace_registry,
            &marketplace_checkout.join("registry"),
        )
        .expect("stage marketplace registry");

        let registry_dir = marketplace_checkout.join("registry");
        let plugin_id = PluginId::parse("official/ora-space.tavily-search").expect("plugin id");
        let manifest = RegistryIndex::resolve_manifest_all(&[registry_dir.as_path()], &plugin_id)
            .expect("resolve marketplace manifest")
            .expect("Tavily listing is present in staged registry");
        assert_eq!(
            manifest.url().map(|url| url.as_url().to_string()),
            Some(
                "https://github.com/ora-space/tavily-search-mcp/releases/download/v0.1.0/ora-space.tavily-search-v0.1.0.orax"
                    .to_string()
            )
        );
        assert_eq!(
            manifest.sha256().map(|digest| digest.to_string()),
            Some("a8b58b0fc0a7c85fe774620682703149b4b6acbaa99303f399309558da282130".to_string())
        );
    }

    /// Downloads and installs Tavily from a staged marketplace registry. Requires the release URL
    /// to be reachable without authentication.
    #[tokio::test]
    #[ignore = "requires ora-space/tavily-search-mcp release assets to be publicly downloadable"]
    async fn tavily_mcp_marketplace_install_and_configuration() {
        use ora_contracts::{
            GetPluginConfigurationRequest, InstallPluginRequest, ListInstalledPluginsRequest,
            PluginConfigurationCompleteness, PluginConfigurationSummary, PluginSettingValue,
            SavePluginConfigurationRequest,
        };
        use pretty_assertions::assert_eq;
        use std::collections::BTreeMap;
        use std::fs;
        use std::path::PathBuf;

        const PLUGIN_ID: &str = "official/ora-space.tavily-search";
        const TEST_API_KEY: &str = "tvly-test-marketplace-e2e";

        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../");
        let marketplace_registry = workspace_root.join(".tmp/marketplace/registry");
        if !marketplace_registry.is_dir() {
            eprintln!(
                "skipping Tavily MCP marketplace E2E: missing {}",
                marketplace_registry.display()
            );
            return;
        }

        let temporary = TempDir::new().expect("create temporary backend directory");
        let data_directory = temporary.path().to_path_buf();
        let marketplace_checkout =
            data_directory.join("plugins/sources/github.com/ora-space/marketplace");
        fs::create_dir_all(&marketplace_checkout).expect("create marketplace checkout");
        copy_dir_recursive(
            &marketplace_registry,
            &marketplace_checkout.join("registry"),
        )
        .expect("stage marketplace registry");

        let backend = Backend::open(BackendPaths {
            database_path: data_directory.join("ora.sqlite3"),
            data_directory: data_directory.clone(),
            deno_path: std::path::PathBuf::from("deno"),
            worktree_root: data_directory.join("worktrees"),
            home_directory: data_directory.clone(),
            relative_path_base: data_directory.clone(),
            sessions_root: data_directory.join("sessions"),
            skills_root: data_directory.join("atoms").join("skills"),
            ripgrep_path: std::path::PathBuf::from("rg"),
            timezone: chrono_tz::UTC,
        })
        .expect("open shared backend");

        backend
            .install_plugin(InstallPluginRequest {
                plugin_id: PLUGIN_ID.to_string(),
            })
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "install Tavily MCP from staged marketplace registry: {error:?}. \
                     If the release URL returns 404, ensure ora-space/tavily-search-mcp is public."
                );
            });

        let installed = backend
            .list_installed_plugins(ListInstalledPluginsRequest {})
            .expect("list installed plugins")
            .plugins
            .into_iter()
            .find(|plugin| plugin.id == PLUGIN_ID)
            .expect("Tavily MCP plugin is installed after marketplace install");
        assert_eq!(
            installed.configuration,
            PluginConfigurationSummary::Available {
                completeness: PluginConfigurationCompleteness::Incomplete,
            }
        );

        let configuration = backend
            .get_plugin_configuration(GetPluginConfigurationRequest {
                plugin_id: PLUGIN_ID.to_string(),
            })
            .expect("load plugin configuration editor")
            .configuration;
        backend
            .save_plugin_configuration(SavePluginConfigurationRequest {
                plugin_id: PLUGIN_ID.to_string(),
                expected_revision: configuration.revision,
                declaration_fingerprint: configuration.declaration_fingerprint.clone(),
                values: BTreeMap::from([(
                    "apiKey".to_string(),
                    PluginSettingValue::String(TEST_API_KEY.to_string()),
                )]),
            })
            .expect("save Tavily apiKey after marketplace install");

        let store_json = fs::read_to_string(
            data_directory.join("plugins/data/official/ora-space.tavily-search/store.json"),
        )
        .expect("read persisted store.json");
        assert!(
            store_json.contains(TEST_API_KEY),
            "store.json should contain the saved apiKey value"
        );
    }

    /// When `ORA_E2E_PLUGIN_DATA` points at a live Desktop plugin home, verifies Tavily settings
    /// persistence against the real on-disk layout.
    #[tokio::test]
    async fn tavily_mcp_save_configuration_in_desktop_plugin_home() {
        use ora_contracts::{
            GetPluginConfigurationRequest, ListInstalledPluginsRequest,
            PluginConfigurationCompleteness, PluginConfigurationSummary, PluginSettingValue,
            SavePluginConfigurationRequest,
        };
        use std::collections::BTreeMap;
        use std::fs;
        use std::path::PathBuf;

        const PLUGIN_ID: &str = "official/ora-space.tavily-search";
        const TEST_API_KEY: &str = "tvly-desktop-e2e-key";

        let Ok(data_directory) = std::env::var("ORA_E2E_PLUGIN_DATA") else {
            return;
        };
        let data_directory = PathBuf::from(data_directory);
        let plugin_data_directory = data_directory.clone();
        if !data_directory.is_dir() {
            eprintln!(
                "skipping desktop-home Tavily E2E: {} is not a directory",
                data_directory.display()
            );
            return;
        }

        let temporary = TempDir::new().expect("create temporary backend directory");
        let backend = Backend::open(BackendPaths {
            database_path: temporary.path().join("ora.sqlite3"),
            data_directory,
            deno_path: std::path::PathBuf::from("deno"),
            worktree_root: temporary.path().join("worktrees"),
            home_directory: temporary.path().to_path_buf(),
            relative_path_base: temporary.path().to_path_buf(),
            sessions_root: temporary.path().join("sessions"),
            skills_root: temporary.path().join("atoms").join("skills"),
            ripgrep_path: std::path::PathBuf::from("rg"),
            timezone: chrono_tz::UTC,
        })
        .expect("open shared backend");

        let installed = backend
            .list_installed_plugins(ListInstalledPluginsRequest {})
            .expect("list installed plugins")
            .plugins
            .into_iter()
            .find(|plugin| plugin.id == PLUGIN_ID)
            .expect("Tavily MCP plugin is installed in desktop plugin home");
        assert!(
            matches!(
                installed.configuration,
                PluginConfigurationSummary::Available {
                    completeness: PluginConfigurationCompleteness::Incomplete,
                } | PluginConfigurationSummary::Available {
                    completeness: PluginConfigurationCompleteness::Complete,
                }
            ),
            "configuration should be available after the dotted-name store-path fix"
        );

        let configuration = backend
            .get_plugin_configuration(GetPluginConfigurationRequest {
                plugin_id: PLUGIN_ID.to_string(),
            })
            .expect("load plugin configuration editor")
            .configuration;
        if matches!(
            configuration.summary,
            PluginConfigurationSummary::Available {
                completeness: PluginConfigurationCompleteness::Complete,
            }
        ) {
            return;
        }
        backend
            .save_plugin_configuration(SavePluginConfigurationRequest {
                plugin_id: PLUGIN_ID.to_string(),
                expected_revision: configuration.revision,
                declaration_fingerprint: configuration.declaration_fingerprint.clone(),
                values: BTreeMap::from([(
                    "apiKey".to_string(),
                    PluginSettingValue::String(TEST_API_KEY.to_string()),
                )]),
            })
            .expect("save Tavily apiKey in desktop plugin home");

        let store_json = fs::read_to_string(
            plugin_data_directory.join("plugins/data/official/ora-space.tavily-search/store.json"),
        )
        .expect("read persisted store.json");
        assert!(
            store_json.contains(TEST_API_KEY),
            "store.json should contain the saved apiKey value"
        );
    }

    /// Recursively copies one directory tree for marketplace registry staging in tests.
    fn copy_dir_recursive(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            let destination = to.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                copy_dir_recursive(&entry.path(), &destination)?;
            } else {
                fs::copy(entry.path(), destination)?;
            }
        }
        Ok(())
    }
}
