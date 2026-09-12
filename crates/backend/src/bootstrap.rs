use crate::agent::AgentApi;
use crate::agent_runtime::{AgentRuntime, AgentRuntimeManager, AgentRuntimeSetup};
use crate::app_event::AppEventHub;
use crate::clock::SystemClock;
use crate::effects::Effects;
use crate::error::BackendError;
use crate::plugin::{PluginApi, Plugins};
use crate::project::ProjectApi;
use crate::session::Sessions;
use crate::settings::Settings;
use crate::skill::SkillApi;
use crate::task::{TaskApi, TaskSetup};
use crate::workflow::WorkflowApi;
use crate::workflow::run::{WorkflowRunSetup, WorkflowRuns};
use crate::workflow::run::{
    build_workflow_run_engine, prune_orphaned_baselines, run_workflow_run_boot_sweep,
};
use crate::workspace::WorkspaceApi;
use ora_application::ApplicationError;
use ora_db::{DatabaseBootstrapper, DatabaseLocation, default_migration_catalog};
use ora_scheduler::Scheduler;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use thiserror::Error;

/// Names the persistent paths required to construct the shared backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendPaths {
    /// Tauri application data root containing SQLite, sessions, and formal Skills.
    pub app_data_directory: PathBuf,
    /// Ora home root (`~/.ora`) containing plugins and worktrees.
    pub home_directory: PathBuf,
    /// Bundled Deno executable used for plugin activation.
    pub deno_path: PathBuf,
    /// Directory against which persisted relative local Workspace locations are resolved.
    ///
    /// Relative locations are stored against the directory from which `ORA_DATA_DIR`
    /// was created. Live process cwd is not used: Desktop `tauri dev` starts in
    /// `src-tauri`, which is not that directory.
    pub relative_path_base: PathBuf,
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
    #[error("persisted worktree root is invalid: {path:?}")]
    InvalidWorktreeRoot { path: PathBuf },
    #[error("failed to load persisted user configuration")]
    UserConfig(#[source] BackendError),
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
    effect: Effects,
    project: Arc<ProjectApi>,
    task: Arc<TaskApi>,
    workspace: WorkspaceApi,
    settings: Arc<Settings>,
    session: Arc<Sessions>,
    agent_runtime: AgentRuntime,
    plugin: Plugins,
    skill: Arc<SkillApi>,
    agent: Arc<AgentApi>,
    workflow: Arc<WorkflowApi>,
    workflow_run: Arc<WorkflowRuns>,
    app_events: Arc<AppEventHub>,
}

impl Backend {
    /// Opens persistent storage and composes domain interfaces with shared runtime ownership.
    ///
    /// Agent providers come from installed plugins discovered under `paths.home_directory`, which
    /// also owns their processes and default worktrees.
    pub fn open(paths: BackendPaths) -> Result<Self, BackendBootstrapError> {
        let database_path = paths.app_data_directory.join("ora.sqlite3");
        let skills_root = paths.app_data_directory.join("atoms").join("skills");
        let sessions_root = paths.app_data_directory.join("sessions");
        let default_worktree_root = paths.home_directory.join("worktrees");
        ensure_directory(&paths.app_data_directory)?;
        let catalog = default_migration_catalog().map_err(BackendBootstrapError::Database)?;
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(&DatabaseLocation::path(&database_path), &catalog)
            .map_err(BackendBootstrapError::Database)?;
        let settings = Arc::new(Settings::new(pool.clone()));
        let stored_worktree_root = settings
            .worktree_root()
            .map_err(BackendBootstrapError::UserConfig)?;
        let configured_worktree_root = match stored_worktree_root {
            Some(root) => {
                if !root.is_absolute() || !root.is_dir() {
                    return Err(BackendBootstrapError::InvalidWorktreeRoot { path: root });
                }
                root
            }
            None => {
                ensure_directory(&default_worktree_root)?;
                default_worktree_root
            }
        };
        crate::skill_reconciliation::reconcile_skill_storage(&pool, &skills_root, &SystemClock)
            .map_err(BackendBootstrapError::SkillStorageReconciliation)?;
        crate::skill_reconciliation::cleanup_import_temp_sessions()
            .map_err(BackendBootstrapError::SkillStorageReconciliation)?;
        let clock = SystemClock;
        let app_events = Arc::new(AppEventHub::new());
        let plugin = Arc::new(
            PluginApi::open(
                pool.clone(),
                paths.home_directory.clone(),
                paths.deno_path,
                clock,
                app_events.publisher(),
                settings.clone(),
            )
            .map_err(BackendBootstrapError::Plugin)?,
        );
        plugin
            .sync_installed_skills()
            .map_err(BackendBootstrapError::PluginSkillCatalog)?;
        let scheduler = Scheduler::new(paths.timezone);
        let worktree_root = Arc::new(RwLock::new(configured_worktree_root));
        // Side files holding the worktree baseline an interactive node diffs at completion.
        let baselines_root = sessions_root.join("node-baselines");
        let relative_path_base = paths.relative_path_base;
        let agent_runtime = Arc::new(
            AgentRuntimeManager::new(AgentRuntimeSetup {
                mcp_selections: Arc::new(
                    crate::workflow::run::WorkflowSessionMcpSelectionSource::new(pool.clone()),
                ),
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
        plugin.set_mcp_wakeup({
            let runtime = agent_runtime.clone();
            Arc::new(move || runtime.notify_mcp_desired_changed())
        });
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
        let effect_worker = crate::effect_worker::EffectWorker::new(
            pool.clone(),
            plugin.clone(),
            agent_runtime.clone(),
        );
        effect_worker.recover();
        // Creating a Workspace is not something a consumer declaration can observe, so both create
        // paths wake the worker to converge it promptly instead of at the next scan.
        let effect_reconcile = effect_worker.spawn();
        plugin.set_effect_reconcile(effect_reconcile.clone());

        let workflow_run = Arc::new(WorkflowRuns::new(WorkflowRunSetup {
            pool: pool.clone(),
            skills_root: skills_root.clone(),
            sessions_root: sessions_root.clone(),
            baselines_root,
            agent_runtime: agent_runtime.clone(),
            engine: workflow_run_engine,
            run_locks,
            clock,
        }));

        let session = Arc::new(Sessions::new(
            pool.clone(),
            agent_runtime.clone(),
            workflow_run.session_turns(),
            app_events.publisher(),
        ));

        Ok(Self {
            project: Arc::new(ProjectApi::new(
                pool.clone(),
                sessions_root.clone(),
                clock,
                effect_reconcile.clone(),
                git_cleanup.clone(),
            )),
            task: Arc::new(TaskApi::new(TaskSetup {
                pool: pool.clone(),
                worktree_root: worktree_root.clone(),
                relative_path_base: relative_path_base.clone(),
                sessions_root,
                repository_gates,
                clock,
                effect_reconcile: effect_reconcile.clone(),
                git_cleanup: git_cleanup.clone(),
            })),
            workspace: WorkspaceApi::new(
                pool.clone(),
                git_cleanup,
                relative_path_base,
                worktree_root,
                settings.clone(),
            ),
            settings,
            session,
            plugin: Plugins::new(plugin, agent_runtime.clone()),
            agent_runtime: AgentRuntime::new(agent_runtime),
            skill: Arc::new(SkillApi::new(
                pool.clone(),
                skills_root,
                clock,
                effect_reconcile,
            )),
            agent: Arc::new(AgentApi::new(pool.clone(), clock)),
            workflow: Arc::new(WorkflowApi::new(pool.clone(), clock)),
            workflow_run,
            app_events,
            effect: Effects::new(pool),
        })
    }

    /// Shares persisted Effect status without exposing convergence workers.
    pub fn effects(&self) -> Effects {
        self.effect.clone()
    }

    /// Shares run use cases, including scheduling serialization and terminal session cleanup.
    pub fn workflow_runs(&self) -> Arc<WorkflowRuns> {
        self.workflow_run.clone()
    }

    /// Returns the settings interface without exposing storage or runtime internals.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Shares configurable-agent use cases without exposing repositories or runtime control.
    pub fn agents(&self) -> Arc<AgentApi> {
        self.agent.clone()
    }

    /// Shares Skill catalog/import use cases, including their Effect convergence obligations.
    pub fn skills(&self) -> Arc<SkillApi> {
        self.skill.clone()
    }

    /// Shares definition/draft/version use cases independently of workflow-run execution.
    pub fn workflows(&self) -> Arc<WorkflowApi> {
        self.workflow.clone()
    }

    /// Shares complete project use cases, including aggregate deletion and cleanup notification.
    pub fn projects(&self) -> Arc<ProjectApi> {
        self.project.clone()
    }

    /// Shares task use cases without exposing Git provisioning gates or deletion transactions.
    pub fn tasks(&self) -> Arc<TaskApi> {
        self.task.clone()
    }

    /// Shares workspace lookup, path configuration, and Git review with the existing use leases.
    pub fn workspaces(&self) -> WorkspaceApi {
        self.workspace.clone()
    }

    /// Shares plugin use cases together with the runtime reconciliation each mutation requires.
    pub fn plugins(&self) -> Plugins {
        self.plugin.clone()
    }

    /// Shares session use cases without exposing actor or workflow synchronization internals.
    pub fn sessions(&self) -> Arc<Sessions> {
        self.session.clone()
    }

    /// Shares the application invalidation source without giving consumers its publisher.
    pub fn app_events(&self) -> Arc<AppEventHub> {
        self.app_events.clone()
    }

    /// Shares readiness and model discovery without exposing supervisor or actor internals.
    pub fn agent_runtime(&self) -> AgentRuntime {
        self.agent_runtime.clone()
    }
}

/// Creates one required runtime directory and preserves its exact failing path.
fn ensure_directory(path: &Path) -> Result<(), BackendBootstrapError> {
    fs::create_dir_all(path).map_err(|source| BackendBootstrapError::DirectoryCreate {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::Backend;
    use crate::error::ErrorClassification;
    use crate::test_backend::backend_paths;
    use ora_contracts::{
        CreateAgentRequest, CreateProjectRequest, CreateSkillRequest, DeleteAgentRequest,
        DeleteProjectRequest, DeleteSkillRequest, GetProjectRequest, ListAgentsRequest,
        ListProjectsRequest, ListSkillsRequest, UpdateAgentRequest, UpdateProjectRequest,
        UpdateSkillRequest,
    };
    use pretty_assertions::assert_eq;
    use std::fs;
    use tempfile::TempDir;

    /// Verifies the shared composition owns storage bootstrap and complete non-Git CRUD flows.
    #[test]
    fn opens_storage_and_serves_shared_crud_apis() {
        ora_logging::with_trace_logging(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test runtime")
                .block_on(async {
                    let temporary = TempDir::new().expect("create temporary backend directory");
                    let database_path = temporary.path().join("data").join("ora.sqlite3");
                    let worktree_root = temporary.path().join("worktrees");
                    let backend = Backend::open(backend_paths(
                        database_path.parent().expect("database has parent"),
                        temporary.path(),
                    ))
                    .expect("open shared backend");

                    assert!(database_path.is_file());
                    assert!(worktree_root.is_dir());

                    let project = backend
                        .projects()
                        .create(CreateProjectRequest {
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
                        .projects()
                        .update(UpdateProjectRequest {
                            project_id: project.id.clone(),
                            name: "Ora Desktop".to_string(),
                        })
                        .expect("update project")
                        .project;
                    assert_eq!(updated_project.name, "Ora Desktop");
                    assert_eq!(
                        backend
                            .projects()
                            .list(ListProjectsRequest {})
                            .expect("list projects")
                            .projects,
                        vec![updated_project.clone()]
                    );

                    let skill = backend
                        .skills()
                        .create(CreateSkillRequest {
                            name: "review".to_string(),
                            description: "Review changes".to_string(),
                            content: None,
                        })
                        .expect("create skill")
                        .skill;
                    let skill = backend
                        .skills()
                        .update(UpdateSkillRequest {
                            skill_id: skill.id,
                            name: "review-code".to_string(),
                            description: "Review implementation changes".to_string(),
                            content: None,
                        })
                        .expect("update skill")
                        .skill;
                    assert_eq!(
                        backend
                            .skills()
                            .list(ListSkillsRequest {})
                            .expect("list skills")
                            .skills,
                        vec![skill.clone()]
                    );

                    let agent = backend
                        .agents()
                        .create(CreateAgentRequest {
                            name: "codex".to_string(),
                            description: "Coding agent".to_string(),
                            content: None,
                        })
                        .expect("create agent")
                        .agent;
                    let agent = backend
                        .agents()
                        .update(UpdateAgentRequest {
                            agent_id: agent.id,
                            name: "codex-desktop".to_string(),
                            description: "Desktop coding agent".to_string(),
                            content: None,
                        })
                        .expect("update agent")
                        .agent;
                    assert_eq!(
                        backend
                            .agents()
                            .list(ListAgentsRequest {})
                            .expect("list agents")
                            .agents,
                        vec![agent.clone()]
                    );

                    backend
                        .agents()
                        .delete(DeleteAgentRequest { agent_id: agent.id })
                        .expect("delete agent");
                    backend
                        .skills()
                        .delete(DeleteSkillRequest { skill_id: skill.id })
                        .expect("delete skill");
                    backend
                        .projects()
                        .delete(DeleteProjectRequest {
                            project_id: project.id.clone(),
                        })
                        .await
                        .expect("delete project");

                    let error = backend
                        .projects()
                        .get(GetProjectRequest {
                            project_id: project.id,
                        })
                        .expect_err("deleted project should be hidden");
                    assert_eq!(error.classification(), ErrorClassification::NotFound);
                    assert_eq!(error.public_error().code(), "project_not_found");
                });
        });
    }

    /// Verifies startup projects installed Skill plugins into the shared Skill catalog.
    #[test]
    fn opens_with_plugin_skills_written_to_the_existing_database_schema() {
        ora_logging::with_trace_logging(|| {
            let temporary = TempDir::new().expect("create temporary backend directory");
            let app_data_directory = temporary.path().join("app-data");
            let home_directory = temporary.path().join("ora-home");
            let package_root = home_directory.join("plugins/installed/official/review-pack/1.0.0");
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

            let backend = Backend::open(backend_paths(&app_data_directory, &home_directory))
                .expect("open shared backend");

            assert!(app_data_directory.join("ora.sqlite3").is_file());
            assert!(home_directory.join("worktrees").is_dir());

            let skills = backend
                .skills()
                .list(ListSkillsRequest {})
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
        });
    }
}
