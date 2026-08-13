use crate::clock::SystemClock;
use crate::git_cleanup::GatedWorktreeProvisioner;
use crate::{BackendError, ErrorClassification};
use gitlancer::git::worktree::ResolveWorktreeByBranchRequest;
use gitlancer::{CliGitRunner, Git, RepoRoot, Repository};
use ora_application::{
    ApplicationError, Clock, CreateTaskHandler, GetTaskHandler, GitTaskWorktreeProvisioner,
    ListTasksHandler, ProjectRepository, RepositoryError, TaskRepository, UpdateTaskHandler,
    UuidTaskIdGenerator, UuidWorktreeIdGenerator, WorktreeRepository,
};
use ora_contracts::{
    CreateTaskRequest, CreateTaskResponse, DeleteTaskRequest, DeleteTaskResponse, GetTaskRequest,
    GetTaskResponse, GetTaskWorkspaceResponse, ListTasksRequest, ListTasksResponse, TaskWorkspace,
    UpdateTaskRequest, UpdateTaskResponse,
};
use ora_contracts::{EmptyErrorParams, PublicError};
use ora_db::{
    CascadeDeleteOutcome, RepositoryPool, SqliteCascadeRepository, SqliteProjectRepository,
    SqliteTaskRepository, SqliteTaskWorkspaceRepository, SqliteWorktreeProvisioningLeaseRepository,
    SqliteWorktreeRepository,
};
use ora_domain::{Project, ProjectId, TaskId, WorktreeActivity};
use ora_logging::ora_warn;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

/// Groups task handlers while resolving each Git repository from the task's owning project.
pub(crate) struct TaskApi {
    pool: RepositoryPool,
    worktree_root: Arc<RwLock<PathBuf>>,
    /// Where cascaded sessions' recorded conversations are removed from.
    sessions_root: PathBuf,
    /// Serializes Git mutations per repository between provisioning and cleanup.
    repository_gates: Arc<crate::git_cleanup::KeyedResourceLocks>,
    get: GetTaskHandler<SqliteTaskRepository>,
    list: ListTasksHandler<SqliteTaskRepository>,
    update: UpdateTaskHandler<SqliteTaskRepository, SystemClock>,
    clock: SystemClock,
}

impl TaskApi {
    /// Builds task handlers from shared persistence and mutable runtime path configuration.
    pub(crate) fn new(
        pool: RepositoryPool,
        worktree_root: Arc<RwLock<PathBuf>>,
        sessions_root: PathBuf,
        repository_gates: Arc<crate::git_cleanup::KeyedResourceLocks>,
        clock: SystemClock,
    ) -> Self {
        let repository = SqliteTaskRepository::new(pool.clone());

        Self {
            pool,
            worktree_root,
            sessions_root,
            repository_gates,
            get: GetTaskHandler::new(repository.clone()),
            list: ListTasksHandler::new(repository.clone()),
            update: UpdateTaskHandler::new(repository, clock),
            clock,
        }
    }

    /// Resolves the requested project and creates its task in the matching Git repository.
    pub(crate) fn create(
        &self,
        request: CreateTaskRequest,
    ) -> Result<CreateTaskResponse, ApplicationError> {
        let project = self.find_project(&ProjectId::new(&request.project_id))?;
        let repository_root = PathBuf::from(project.root_path);
        let handler = CreateTaskHandler::new(
            SqliteTaskWorkspaceRepository::new(self.pool.clone()),
            SqliteWorktreeProvisioningLeaseRepository::new(self.pool.clone()),
            UuidTaskIdGenerator::new(),
            UuidWorktreeIdGenerator::new(),
            GatedWorktreeProvisioner::new(
                GitTaskWorktreeProvisioner::new(repository_root.clone()),
                Arc::clone(&self.repository_gates),
                crate::git_cleanup::normalize_repository_key(&repository_root),
            ),
            repository_root,
            self.worktree_root_snapshot()?,
            self.clock,
        );

        handler.handle(request)
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
        let session_ids = crate::session_history::session_ids_for_task(&self.pool, &task_id);
        let outcome = SqliteCascadeRepository::new(self.pool.clone())
            .delete_task(&task_id, self.clock.now_timestamp_millis())
            .map_err(|source| BackendError::internal("task repository operation failed", source))?;

        match outcome {
            CascadeDeleteOutcome::Deleted => {
                crate::session_history::remove_session_histories(&self.sessions_root, session_ids);
                Ok(DeleteTaskResponse {
                    task_id: task_id.to_string(),
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

/// Lists every visible task belonging to one project.
///
/// Read before a cascading project delete, which soft-deletes those rows and
/// leaves this query with nothing to report afterwards. A failure here yields an
/// empty list rather than an error: the caller uses it to clean up warm sessions
/// the deleted project owned, and failing the user's delete over a cleanup query
/// would be the larger harm.
pub(crate) fn task_ids_in_project(pool: &RepositoryPool, project_id: &ProjectId) -> Vec<TaskId> {
    match SqliteTaskRepository::new(pool.clone()).list_tasks() {
        Ok(tasks) => tasks
            .into_iter()
            .filter(|task| &task.project_id == project_id)
            .map(|task| task.id)
            .collect(),
        Err(_) => {
            ora_warn!(
                project_id = %project_id,
                "listing project tasks for warm session cleanup failed",
            );
            Vec::new()
        }
    }
}

/// Resolves the task's authoritative execution directory from its selected workspace mode.
pub(crate) fn resolve_task_cwd(
    pool: &RepositoryPool,
    task_id: &TaskId,
    relative_path_base: &Path,
) -> Result<PathBuf, BackendError> {
    let task = SqliteTaskRepository::new(pool.clone())
        .find_task(task_id)
        .map_err(task_worktree_unavailable_with)?
        .ok_or_else(task_worktree_unavailable)?;
    if task.worktree_id.is_none() {
        let project = SqliteProjectRepository::new(pool.clone())
            .find_project(&task.project_id)
            .map_err(task_project_root_unavailable_with)?
            .ok_or_else(task_project_root_unavailable)?;
        return absolute_project_root(PathBuf::from(project.root_path), relative_path_base);
    }

    let worktree_id = task.worktree_id.ok_or_else(task_worktree_unavailable)?;
    let worktree = SqliteWorktreeRepository::new(pool.clone())
        .find_worktree(&worktree_id)
        .map_err(task_worktree_unavailable_with)?
        .ok_or_else(task_worktree_unavailable)?;
    if worktree.task_id != task.id || worktree.activity != WorktreeActivity::Active {
        return Err(task_worktree_unavailable());
    }
    let branch_name = worktree.branch_name.ok_or_else(task_worktree_unavailable)?;
    let project = SqliteProjectRepository::new(pool.clone())
        .find_project(&task.project_id)
        .map_err(task_worktree_unavailable_with)?
        .ok_or_else(task_worktree_unavailable)?;
    let repository = Repository::new(RepoRoot::new(project.root_path));
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

/// Resolves the same task root used by agent sessions and reports a branch only for linked worktrees.
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
    let branch_name = match task.worktree_id {
        Some(worktree_id) => SqliteWorktreeRepository::new(pool.clone())
            .find_worktree(&worktree_id)
            .map_err(task_worktree_unavailable_with)?
            .filter(|worktree| worktree.activity == WorktreeActivity::Active)
            .and_then(|worktree| worktree.branch_name),
        None => None,
    };
    let root = resolve_task_cwd(pool, &task_id, relative_path_base)?;
    Ok(GetTaskWorkspaceResponse {
        workspace: TaskWorkspace {
            root_path: root.to_string_lossy().into_owned(),
            branch_name,
        },
    })
}

/// Resolves the execution directory for a chat whose Task does not exist yet.
///
/// Direct chats create their Task only when the first message is sent, but the
/// model selector needs a session before that. Those Tasks are always created in
/// project-root mode, so the project root resolved here matches the directory
/// the eventual Task resolves to.
pub(crate) fn resolve_project_cwd(
    pool: &RepositoryPool,
    project_id: &ProjectId,
    relative_path_base: &Path,
) -> Result<PathBuf, BackendError> {
    let project = SqliteProjectRepository::new(pool.clone())
        .find_project(project_id)
        .map_err(|_| task_project_root_unavailable())?
        .ok_or_else(task_project_root_unavailable)?;
    absolute_project_root(PathBuf::from(project.root_path), relative_path_base)
}

/// Resolves a persisted project root against the runtime's stable path base.
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
        Err(task_project_root_unavailable())
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

/// Builds the conflict used when a project-root task no longer has a usable directory.
fn task_project_root_unavailable() -> BackendError {
    BackendError::new(
        ErrorClassification::Conflict,
        PublicError::TaskProjectRootUnavailable(EmptyErrorParams {}),
        "task project root is unavailable",
    )
}

fn task_project_root_unavailable_with(
    source: impl std::error::Error + Send + Sync + 'static,
) -> BackendError {
    BackendError::with_source(
        ErrorClassification::Conflict,
        PublicError::TaskProjectRootUnavailable(EmptyErrorParams {}),
        "task project root is unavailable",
        source,
    )
}

#[cfg(test)]
mod tests {
    use super::{absolute_project_root, get_task_workspace, resolve_task_cwd};
    use ora_application::{ProjectRepository, TaskRepository};
    use ora_contracts::{GetTaskWorkspaceResponse, TaskWorkspace};
    use ora_db::{
        DatabaseBootstrapper, DatabaseLocation, SqliteProjectRepository, SqliteTaskRepository,
        default_migration_catalog,
    };
    use ora_domain::{AuditFields, Project, ProjectId, Task, TaskId, TaskStatus};
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Verifies direct-chat tasks start providers in the project root without a worktree link.
    #[test]
    fn resolves_project_root_for_tasks_without_worktrees() {
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
            .create_project(Project::new(
                ProjectId::new("project-1"),
                "Project",
                project_root.to_string_lossy(),
                AuditFields::new(1, 1, false),
            ))
            .expect("persist project");
        SqliteTaskRepository::new(pool.clone())
            .create_task(Task::new(
                TaskId::new("task-1"),
                ProjectId::new("project-1"),
                "Project chat",
                TaskStatus::Doing,
                None,
                AuditFields::new(1, 1, false),
            ))
            .expect("persist task");

        assert_eq!(
            resolve_task_cwd(&pool, &TaskId::new("task-1"), temp_dir.path())
                .expect("resolve project root cwd"),
            project_root.clone(),
        );
        assert_eq!(
            get_task_workspace(&pool, "task-1", temp_dir.path())
                .expect("load project-root workspace"),
            GetTaskWorkspaceResponse {
                workspace: TaskWorkspace {
                    root_path: project_root.to_string_lossy().into_owned(),
                    branch_name: None,
                },
            },
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
        assert_eq!(error.to_string(), "task project root is unavailable");
    }
}
