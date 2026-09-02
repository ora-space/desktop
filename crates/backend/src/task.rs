use crate::clock::SystemClock;
use crate::effect_worker::EffectWorkerHandle;
use crate::git_cleanup::GatedWorktreeProvisioner;
use crate::{BackendError, ErrorClassification};
use gitlancer::git::worktree::ResolveWorktreeByBranchRequest;
use gitlancer::{CliGitRunner, Git, RepoRoot, Repository};
use ora_application::{
    ApplicationError, Clock, CreateTaskHandler, GetTaskHandler, GitTaskWorktreeProvisioner,
    ListTasksHandler, ProjectRepository, RepositoryError, TaskRepository, UpdateTaskHandler,
    UuidTaskIdGenerator, WorktreeRepository,
};
use ora_contracts::{
    CreateTaskRequest, CreateTaskResponse, DeleteTaskRequest, DeleteTaskResponse, GetTaskRequest,
    GetTaskResponse, GetTaskWorkspaceResponse, ListTasksRequest, ListTasksResponse, TaskWorkspace,
    UpdateTaskRequest, UpdateTaskResponse,
};
use ora_contracts::{EmptyErrorParams, PublicError};
use ora_db::{
    CascadeDeleteOutcome, RepositoryPool, SqliteCascadeRepository, SqliteProjectRepository,
    SqliteTaskRepository, SqliteTaskWorkspaceRepository, SqliteWorkspaceRepository,
    SqliteWorktreeProvisioningLeaseRepository, SqliteWorktreeRepository,
};
use ora_domain::{Project, ProjectId, TaskId, WorkspaceId, WorkspaceLocation, WorktreeActivity};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

/// Groups task handlers while resolving each Git repository from the task's owning project.
pub(crate) struct TaskApi {
    pool: RepositoryPool,
    worktree_root: Arc<RwLock<PathBuf>>,
    /// Base directory used to resolve relative main Workspace locations.
    relative_path_base: PathBuf,
    /// Where cascaded sessions' recorded conversations are removed from.
    sessions_root: PathBuf,
    /// Serializes Git mutations per repository between provisioning and cleanup.
    repository_gates: Arc<crate::git_cleanup::KeyedResourceLocks>,
    get: GetTaskHandler<SqliteTaskRepository>,
    list: ListTasksHandler<SqliteTaskRepository>,
    update: UpdateTaskHandler<SqliteTaskRepository, SystemClock>,
    clock: SystemClock,
    /// Wakes Effect convergence for the Workspace a new task brings with it.
    effect_reconcile: EffectWorkerHandle,
}

impl TaskApi {
    /// Builds task handlers from shared persistence and mutable runtime path configuration.
    pub(crate) fn new(
        pool: RepositoryPool,
        worktree_root: Arc<RwLock<PathBuf>>,
        relative_path_base: PathBuf,
        sessions_root: PathBuf,
        repository_gates: Arc<crate::git_cleanup::KeyedResourceLocks>,
        clock: SystemClock,
        effect_reconcile: EffectWorkerHandle,
    ) -> Self {
        let repository = SqliteTaskRepository::new(pool.clone());

        Self {
            pool,
            worktree_root,
            relative_path_base,
            sessions_root,
            repository_gates,
            effect_reconcile,
            get: GetTaskHandler::new(repository.clone()),
            list: ListTasksHandler::new(repository.clone()),
            update: UpdateTaskHandler::new(repository, clock),
            clock,
        }
    }

    /// Resolves the requested project and creates its task in the matching Git repository.
    ///
    /// The task Workspace needs the Effect Targets every running Consumer already declared, and
    /// no declaration will fire again on its own. Waking here is a latency optimization only: the
    /// worker converges the same Workspace within one scan interval regardless, so a wake lost to a
    /// crash costs a scan interval rather than the materialization.
    pub(crate) fn create(
        &self,
        request: CreateTaskRequest,
    ) -> Result<CreateTaskResponse, ApplicationError> {
        let project_id = ProjectId::new(&request.project_id);
        self.find_project(&project_id)?;
        let repository_root = self.find_main_workspace_root(&project_id)?;
        let handler = CreateTaskHandler::new(
            SqliteTaskWorkspaceRepository::new(self.pool.clone()),
            SqliteWorktreeProvisioningLeaseRepository::new(self.pool.clone()),
            UuidTaskIdGenerator::new(),
            GatedWorktreeProvisioner::new(
                GitTaskWorktreeProvisioner::new(repository_root.clone()),
                Arc::clone(&self.repository_gates),
                crate::git_cleanup::normalize_repository_key(&repository_root),
            ),
            repository_root,
            self.worktree_root_snapshot()?,
            self.clock,
        );

        let response = handler.handle(request)?;
        self.effect_reconcile.notify();
        Ok(response)
    }

    /// Executes one task lookup through the application handler.
    pub(crate) fn get(&self, request: GetTaskRequest) -> Result<GetTaskResponse, ApplicationError> {
        self.get.handle(request)
    }

    /// Executes task listing through the application handler.
    pub(crate) fn list(
        &self,
        request: ListTasksRequest,
    ) -> Result<ListTasksResponse, ApplicationError> {
        self.list.handle(request)
    }

    /// Executes task replacement while preserving its owning project.
    pub(crate) fn update(
        &self,
        request: UpdateTaskRequest,
    ) -> Result<UpdateTaskResponse, ApplicationError> {
        self.update.handle(request)
    }

    /// Soft-deletes the task and Ora worktree record without touching Git state.
    pub(crate) fn delete(
        &self,
        request: DeleteTaskRequest,
    ) -> Result<DeleteTaskResponse, BackendError> {
        let task_id = TaskId::new(request.task_id);
        // Collected before the cascade: once the rows are soft-deleted nothing
        // links their history files back to the task that owned them.
        let workspace_id = SqliteTaskRepository::new(self.pool.clone())
            .find_task(&task_id)
            .map_err(|source| BackendError::internal("task repository operation failed", source))?
            .map(|task| task.workspace_id)
            .ok_or_else(|| {
                BackendError::new(
                    ErrorClassification::NotFound,
                    PublicError::TaskNotFound(EmptyErrorParams {}),
                    format!("task not found: {task_id}"),
                )
            })?;
        let session_ids =
            crate::session_history::session_ids_for_workspace(&self.pool, &workspace_id);
        let outcome = SqliteCascadeRepository::new(self.pool.clone())
            .delete_task(&task_id, self.clock.now_timestamp_millis())
            .map_err(|source| BackendError::internal("task repository operation failed", source))?;

        match outcome {
            CascadeDeleteOutcome::Deleted => {
                crate::session_history::remove_session_histories(&self.sessions_root, session_ids);
                Ok(DeleteTaskResponse {
                    task_id: task_id.to_string(),
                    workspace_id: workspace_id.to_string(),
                })
            }
            CascadeDeleteOutcome::NotFound => Err(BackendError::new(
                ErrorClassification::NotFound,
                PublicError::TaskNotFound(EmptyErrorParams {}),
                format!("task not found: {task_id}"),
            )),
            CascadeDeleteOutcome::ActiveSession => Err(BackendError::new(
                ErrorClassification::Conflict,
                PublicError::ResourceInUse(EmptyErrorParams {}),
                "task has a running session and cannot be deleted",
            )),
        }
    }

    /// Loads a visible project or returns the same stable not-found error as project handlers.
    fn find_project(&self, project_id: &ProjectId) -> Result<Project, ApplicationError> {
        let repository = SqliteProjectRepository::new(self.pool.clone());
        let project = repository
            .find_project(project_id)
            .map_err(project_repository_error)?;

        project.ok_or_else(|| ApplicationError::ProjectNotFound {
            project_id: project_id.to_string(),
        })
    }

    /// Resolves the Git repository from the project's main workspace location.
    fn find_main_workspace_root(
        &self,
        project_id: &ProjectId,
    ) -> Result<PathBuf, ApplicationError> {
        let workspace = SqliteWorkspaceRepository::new(self.pool.clone())
            .find_main_workspace(project_id)
            .map_err(|error| project_repository_error(RepositoryError::new(error)))?
            .ok_or_else(|| ApplicationError::ProjectNotFound {
                project_id: project_id.to_string(),
            })?;
        let WorkspaceLocation::LocalFilesystem { path } = workspace.location else {
            return Err(ApplicationError::TaskWorktreeRootUnavailable);
        };
        absolute_project_root(PathBuf::from(path), &self.relative_path_base)
            .map_err(|_| ApplicationError::TaskWorktreeRootUnavailable)
    }

    /// Captures the configured creation root once so an in-flight operation remains coherent.
    fn worktree_root_snapshot(&self) -> Result<PathBuf, ApplicationError> {
        self.worktree_root
            .read()
            .map(|root| root.clone())
            .map_err(|_poisoned| ApplicationError::TaskWorktreeRootUnavailable)
    }
}

/// Converts project repository failures encountered during dynamic task routing.
fn project_repository_error(error: RepositoryError) -> ApplicationError {
    ApplicationError::ProjectRepository { source: error }
}

/// Resolves the task's authoritative execution directory from its linked worktree.
pub(crate) fn resolve_task_cwd(
    pool: &RepositoryPool,
    task_id: &TaskId,
    relative_path_base: &Path,
) -> Result<PathBuf, BackendError> {
    let task = SqliteTaskRepository::new(pool.clone())
        .find_task(task_id)
        .map_err(task_worktree_unavailable_with)?
        .ok_or_else(task_worktree_unavailable)?;
    let worktree = SqliteWorktreeRepository::new(pool.clone())
        .find_worktree(&task.workspace_id)
        .map_err(task_worktree_unavailable_with)?
        .ok_or_else(task_worktree_unavailable)?;
    if worktree.activity != WorktreeActivity::Active {
        return Err(task_worktree_unavailable());
    }
    let branch_name = worktree.branch_name.ok_or_else(task_worktree_unavailable)?;
    let workspace = SqliteWorkspaceRepository::new(pool.clone())
        .find_main_workspace(&task.project_id)
        .map_err(task_worktree_unavailable_with)?
        .ok_or_else(task_worktree_unavailable)?;
    let WorkspaceLocation::LocalFilesystem { path } = workspace.location else {
        return Err(task_worktree_unavailable());
    };
    let repository_root = absolute_project_root(PathBuf::from(path), relative_path_base)?;
    let repository = Repository::new(RepoRoot::new(repository_root));
    let resolved = Git::new(CliGitRunner)
        .resolve_worktree_by_branch(ResolveWorktreeByBranchRequest {
            repository: &repository,
            branch_name: &branch_name,
        })
        .map_err(task_worktree_unavailable_with)?;
    let cwd = resolved.worktree_root().as_path().to_path_buf();
    if !cwd.is_dir() {
        return Err(task_worktree_unavailable());
    }
    Ok(cwd)
}

/// Resolves an active workspace location to the directory used by its provider session.
pub(crate) fn resolve_workspace_cwd(
    pool: &RepositoryPool,
    workspace_id: &WorkspaceId,
    relative_path_base: &Path,
) -> Result<PathBuf, BackendError> {
    let workspace = SqliteWorkspaceRepository::new(pool.clone())
        .find_workspace(workspace_id)
        .map_err(workspace_unavailable_with)?
        .ok_or_else(workspace_unavailable)?;
    if !workspace.is_admissible() {
        return Err(workspace_unavailable());
    }
    let WorkspaceLocation::LocalFilesystem { path } = workspace.location else {
        return Err(workspace_unavailable());
    };
    absolute_project_root(PathBuf::from(path), relative_path_base)
}

/// Resolves the task-owned worktree root and its branch for the task workspace endpoint.
pub(crate) fn get_task_workspace(
    pool: &RepositoryPool,
    task_id: &str,
    relative_path_base: &Path,
) -> Result<GetTaskWorkspaceResponse, BackendError> {
    let task_id = TaskId::new(task_id);
    let task = SqliteTaskRepository::new(pool.clone())
        .find_task(&task_id)
        .map_err(|source| BackendError::internal("task repository operation failed", source))?
        .ok_or_else(|| {
            BackendError::new(
                ErrorClassification::NotFound,
                PublicError::TaskNotFound(EmptyErrorParams {}),
                format!("task not found: {task_id}"),
            )
        })?;
    let branch_name = SqliteWorktreeRepository::new(pool.clone())
        .find_worktree(&task.workspace_id)
        .map_err(task_worktree_unavailable_with)?
        .filter(|worktree| worktree.activity == WorktreeActivity::Active)
        .and_then(|worktree| worktree.branch_name)
        .ok_or_else(task_worktree_unavailable)?;
    let root = resolve_task_cwd(pool, &task_id, relative_path_base)?;
    Ok(GetTaskWorkspaceResponse {
        workspace: TaskWorkspace {
            root_path: root.to_string_lossy().into_owned(),
            branch_name: Some(branch_name),
        },
    })
}

/// Resolves the main workspace directory for an ordinary project chat.
pub(crate) fn resolve_project_cwd(
    pool: &RepositoryPool,
    project_id: &ProjectId,
    relative_path_base: &Path,
) -> Result<PathBuf, BackendError> {
    let workspace = SqliteWorkspaceRepository::new(pool.clone())
        .find_main_workspace(project_id)
        .map_err(|_| workspace_unavailable())?
        .ok_or_else(workspace_unavailable)?;
    let WorkspaceLocation::LocalFilesystem { path } = workspace.location else {
        return Err(workspace_unavailable());
    };
    absolute_project_root(PathBuf::from(path), relative_path_base)
}

/// Resolves a persisted main Workspace location against the runtime's stable path base.
///
/// Relative roots are stored against the directory from which `ORA_DATA_DIR` was
/// created. Joining them to a live process cwd would miss those directories
/// whenever the binary is started elsewhere — Desktop `tauri dev` starts in
/// `src-tauri`.
pub(crate) fn absolute_project_root(
    path: PathBuf,
    relative_path_base: &Path,
) -> Result<PathBuf, BackendError> {
    let cwd = if path.is_absolute() {
        path
    } else {
        relative_path_base.join(path)
    };
    if cwd.is_dir() {
        Ok(cwd)
    } else {
        Err(workspace_unavailable())
    }
}

/// Builds the conflict used when task ownership cannot resolve an active Git worktree.
fn task_worktree_unavailable() -> BackendError {
    BackendError::new(
        ErrorClassification::Conflict,
        PublicError::TaskWorktreeUnavailable(EmptyErrorParams {}),
        "task worktree is unavailable",
    )
}

fn task_worktree_unavailable_with(
    source: impl std::error::Error + Send + Sync + 'static,
) -> BackendError {
    BackendError::with_source(
        ErrorClassification::Conflict,
        PublicError::TaskWorktreeUnavailable(EmptyErrorParams {}),
        "task worktree is unavailable",
        source,
    )
}

/// Builds the conflict used when a workspace has no usable local directory.
pub(crate) fn workspace_unavailable() -> BackendError {
    BackendError::new(
        ErrorClassification::Conflict,
        PublicError::WorkspaceUnavailable(EmptyErrorParams {}),
        "workspace is unavailable",
    )
}

/// Preserves the repository cause while reporting a workspace-scoped admission failure.
fn workspace_unavailable_with(
    source: impl std::error::Error + Send + Sync + 'static,
) -> BackendError {
    BackendError::with_source(
        ErrorClassification::Conflict,
        PublicError::WorkspaceUnavailable(EmptyErrorParams {}),
        "workspace is unavailable",
        source,
    )
}

#[cfg(test)]
mod tests {
    use super::{absolute_project_root, resolve_project_cwd};
    use ora_application::ProjectRepository;
    use ora_db::{
        DatabaseBootstrapper, DatabaseLocation, SqliteProjectRepository, default_migration_catalog,
    };
    use ora_domain::{AuditFields, Project, ProjectId, WorkspaceLocation};
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Verifies ordinary project chats resolve the project's main workspace directly.
    #[test]
    fn resolves_main_workspace_for_project_chats() {
        let temp_dir = TempDir::new().expect("create temporary directory");
        let project_root = temp_dir.path().join("project-root");
        fs::create_dir_all(&project_root).expect("create project root");
        let database_path = temp_dir.path().join("ora.sqlite3");
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(&database_path),
                &default_migration_catalog().expect("create migration catalog"),
            )
            .expect("bootstrap repository pool");
        SqliteProjectRepository::new(pool.clone())
            .create_project(
                Project::new(
                    ProjectId::new("project-1"),
                    "Project",
                    AuditFields::new(1, 1, false),
                ),
                WorkspaceLocation::local_filesystem(project_root.to_string_lossy()),
            )
            .expect("persist project");
        assert_eq!(
            resolve_project_cwd(&pool, &ProjectId::new("project-1"), temp_dir.path())
                .expect("resolve main workspace cwd"),
            project_root.clone(),
        );
    }

    /// Verifies relative roots are resolved against the injected base, not process cwd.
    #[test]
    fn resolves_relative_project_roots_against_injected_base() {
        let base = TempDir::new().expect("create path base");
        let project_root = base.path().join("nested").join("repo");
        fs::create_dir_all(&project_root).expect("create nested project root");

        let resolved = absolute_project_root(PathBuf::from("nested").join("repo"), base.path())
            .expect("resolve relative project root");

        assert_eq!(resolved, project_root);
        assert!(resolved.is_absolute());
    }

    /// Verifies a relative root that does not exist under the injected base is rejected.
    #[test]
    fn rejects_relative_project_root_missing_from_injected_base() {
        let base = TempDir::new().expect("create path base");
        let error = absolute_project_root(PathBuf::from("missing-project"), base.path())
            .expect_err("missing relative root");
        assert_eq!(error.to_string(), "workspace is unavailable");
    }
}
