use crate::agent::AgentApi;
use crate::agent_runtime::{AgentRuntimeManager, SessionEventStream};
use crate::clock::SystemClock;
use crate::error::{BackendError, BackendErrorKind};
use crate::project::ProjectApi;
use crate::session::SessionApi;
use crate::skill::SkillApi;
use crate::task::TaskApi;
use ora_contracts::*;
use ora_db::{DatabaseBootstrapper, DatabaseLocation, RepositoryPool, default_migration_catalog};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use thiserror::Error;

/// Names the persistent paths required to construct the shared backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendPaths {
    pub database_path: PathBuf,
    pub worktree_root: PathBuf,
    pub home_directory: PathBuf,
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
    #[error("failed to initialize agent runtime")]
    AgentRuntime(#[source] BackendError),
}

/// Owns the concrete persisted use-case composition shared by Web and Tauri adapters.
#[derive(Clone)]
pub struct Backend {
    pool: RepositoryPool,
    worktree_root: Arc<RwLock<PathBuf>>,
    project: Arc<ProjectApi>,
    task: Arc<TaskApi>,
    session: Arc<SessionApi>,
    agent_runtime: Arc<AgentRuntimeManager>,
    skill: Arc<SkillApi>,
    agent: Arc<AgentApi>,
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
        let clock = SystemClock;
        let worktree_root = Arc::new(RwLock::new(paths.worktree_root));
        let agent_runtime = AgentRuntimeManager::new(pool.clone(), paths.home_directory, clock)
            .map_err(BackendBootstrapError::AgentRuntime)?;

        Ok(Self {
            project: Arc::new(ProjectApi::new(pool.clone(), clock)),
            task: Arc::new(TaskApi::new(pool.clone(), worktree_root.clone(), clock)),
            session: Arc::new(SessionApi::new(pool.clone())),
            agent_runtime: Arc::new(agent_runtime),
            skill: Arc::new(SkillApi::new(pool.clone(), clock)),
            agent: Arc::new(AgentApi::new(pool.clone(), clock)),
            pool,
            worktree_root,
        })
    }

    /// Returns the repository pool needed by server-only services excluded from this extraction.
    pub fn repository_pool(&self) -> RepositoryPool {
        self.pool.clone()
    }

    /// Replaces the root used by task creations that start after this update.
    pub fn set_worktree_root(&self, worktree_root: PathBuf) -> Result<(), BackendError> {
        let mut configured_root = self.worktree_root.write().map_err(|_| {
            BackendError::new(
                BackendErrorKind::Internal,
                "worktree_configuration_error",
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
        crate::agent_runtime::resolve_task_cwd(&self.pool, &ora_domain::TaskId::new(task_id))
    }

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
    /// Updates one project through the shared application composition.
    pub fn update_project(
        &self,
        request: UpdateProjectRequest,
    ) -> Result<UpdateProjectResponse, BackendError> {
        self.project.update(request).map_err(BackendError::from)
    }
    /// Deletes one project through the shared application composition.
    pub fn delete_project(
        &self,
        request: DeleteProjectRequest,
    ) -> Result<DeleteProjectResponse, BackendError> {
        self.project.delete(request)
    }

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
    pub fn delete_task(
        &self,
        request: DeleteTaskRequest,
    ) -> Result<DeleteTaskResponse, BackendError> {
        self.task.delete(request)
    }

    /// Creates one session through the shared application composition.
    pub async fn create_session(
        &self,
        request: CreateSessionRequest,
    ) -> Result<CreateSessionResponse, BackendError> {
        self.agent_runtime.create_session(request).await
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
    /// Streams the provider-owned history for one persisted session.
    pub async fn load_session(
        &self,
        request: LoadSessionRequest,
    ) -> Result<SessionEventStream<LoadSessionEvent>, BackendError> {
        self.agent_runtime.load_session(request).await
    }

    /// Streams one text-only prompt turn for a running session.
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

    /// Stops a running provider process while retaining its loadable Ora record.
    pub async fn stop_session(
        &self,
        request: StopSessionRequest,
    ) -> Result<StopSessionResponse, BackendError> {
        self.agent_runtime.stop_session(request).await
    }

    /// Stops one session before removing only its Ora-owned record.
    pub async fn delete_session(
        &self,
        request: DeleteSessionRequest,
    ) -> Result<DeleteSessionResponse, BackendError> {
        self.agent_runtime.delete_session(&request.session_id).await
    }

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
    use super::{Backend, BackendPaths};
    use crate::error::BackendErrorKind;
    use ora_contracts::CreateTaskRequest;
    use ora_contracts::{
        CreateAgentRequest, CreateProjectRequest, CreateSkillRequest, DeleteAgentRequest,
        DeleteProjectRequest, DeleteSkillRequest, DeleteTaskRequest, GetProjectRequest,
        GetTaskRequest, ListAgentsRequest, ListProjectsRequest, ListSkillsRequest, TaskStatus,
        UpdateAgentRequest, UpdateProjectRequest, UpdateSkillRequest,
    };
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use tempfile::TempDir;

    /// Verifies the shared composition owns storage bootstrap and complete non-Git CRUD flows.
    #[test]
    fn opens_storage_and_serves_shared_crud_apis() {
        let temporary = TempDir::new().expect("create temporary backend directory");
        let database_path = temporary.path().join("data").join("ora.sqlite3");
        let worktree_root = temporary.path().join("worktrees");
        let backend = Backend::open(BackendPaths {
            database_path: database_path.clone(),
            worktree_root: worktree_root.clone(),
            home_directory: temporary.path().to_path_buf(),
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
            })
            .expect("create skill")
            .skill;
        let skill = backend
            .update_skill(UpdateSkillRequest {
                skill_id: skill.id,
                name: "review-code".to_string(),
                description: "Review implementation changes".to_string(),
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
            })
            .expect("create agent")
            .agent;
        let agent = backend
            .update_agent(UpdateAgentRequest {
                agent_id: agent.id,
                name: "codex-desktop".to_string(),
                description: "Desktop coding agent".to_string(),
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
            .expect("delete project");

        let error = backend
            .get_project(GetProjectRequest {
                project_id: project.id,
            })
            .expect_err("deleted project should be hidden");
        assert_eq!(error.kind(), BackendErrorKind::NotFound);
        assert_eq!(error.code(), "project_not_found");
    }

    /// Verifies task deletion hides Ora records while deliberately preserving the Git worktree.
    #[test]
    fn deletes_existing_task_after_worktree_root_changes() {
        let temporary = TempDir::new().expect("create temporary backend directory");
        let repository_root = temporary.path().join("repository");
        initialize_repository(&repository_root);
        let original_worktree_root = temporary.path().join("original-worktrees");
        let backend = Backend::open(BackendPaths {
            database_path: temporary.path().join("ora.sqlite3"),
            worktree_root: original_worktree_root.clone(),
            home_directory: temporary.path().to_path_buf(),
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
                status: TaskStatus::Todo,
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
