use crate::clock::SystemClock;
use crate::{BackendError, BackendErrorKind};
use gitlancer::git::worktree::ResolveWorktreeByBranchRequest;
use gitlancer::{CliGitRunner, Git, RepoRoot, Repository};
use ora_application::{
    ApplicationError, Clock, CreateTaskHandler, GetTaskHandler, GitTaskWorktreeProvisioner,
    ListTasksHandler, ProjectRepository, ProjectRepositoryError, TaskRepository, UpdateTaskHandler,
    UuidTaskIdGenerator, UuidWorktreeIdGenerator, WorktreeRepository,
};
use ora_contracts::{
    CreateTaskRequest, CreateTaskResponse, DeleteTaskRequest, DeleteTaskResponse, GetTaskRequest,
    GetTaskResponse, ListTasksRequest, ListTasksResponse, UpdateTaskRequest, UpdateTaskResponse,
};
use ora_db::{
    CascadeDeleteOutcome, RepositoryPool, SqliteCascadeRepository, SqliteProjectRepository,
    SqliteTaskRepository, SqliteWorktreeRepository,
};
use ora_domain::{Project, ProjectId, TaskId, WorktreeActivity};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// Groups task handlers while resolving each Git repository from the task's owning project.
pub(crate) struct TaskApi {
    pool: RepositoryPool,
    worktree_root: Arc<RwLock<PathBuf>>,
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
        clock: SystemClock,
    ) -> Self {
        let repository = SqliteTaskRepository::new(pool.clone());

        Self {
            pool,
            worktree_root,
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
        let task_repository = SqliteTaskRepository::new(self.pool.clone());
        let worktree_repository = SqliteWorktreeRepository::new(self.pool.clone());
        let handler = CreateTaskHandler::new(
            task_repository,
            worktree_repository,
            UuidTaskIdGenerator::new(),
            UuidWorktreeIdGenerator::new(),
            GitTaskWorktreeProvisioner::new(PathBuf::from(project.root_path)),
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
        let outcome = SqliteCascadeRepository::new(self.pool.clone())
            .delete_task(&task_id, self.clock.now_timestamp_millis())
            .map_err(|_| {
                BackendError::new(
                    BackendErrorKind::Internal,
                    "task_repository_error",
                    "task repository operation failed",
                )
            })?;

        match outcome {
            CascadeDeleteOutcome::Deleted => Ok(DeleteTaskResponse {
                task_id: task_id.to_string(),
            }),
            CascadeDeleteOutcome::NotFound => Err(BackendError::new(
                BackendErrorKind::NotFound,
                "task_not_found",
                format!("task not found: {task_id}"),
            )),
            CascadeDeleteOutcome::ActiveSession => Err(BackendError::new(
                BackendErrorKind::Conflict,
                "resource_in_use",
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
            .map_err(|_| ApplicationError::TaskWorktree {
                message: "worktree root configuration is unavailable".to_string(),
            })
    }
}

/// Converts project repository failures encountered during dynamic task routing.
fn project_repository_error(error: ProjectRepositoryError) -> ApplicationError {
    match error {
        ProjectRepositoryError::OperationFailed(message) => {
            ApplicationError::ProjectRepository { message }
        }
    }
}

/// Resolves the task's authoritative execution directory from its selected workspace mode.
pub(crate) fn resolve_task_cwd(
    pool: &RepositoryPool,
    task_id: &TaskId,
) -> Result<PathBuf, BackendError> {
    let task = SqliteTaskRepository::new(pool.clone())
        .find_task(task_id)
        .map_err(|_| task_worktree_unavailable())?
        .ok_or_else(task_worktree_unavailable)?;
    if task.worktree_id.is_none() {
        let project = SqliteProjectRepository::new(pool.clone())
            .find_project(&task.project_id)
            .map_err(|_| task_project_root_unavailable())?
            .ok_or_else(task_project_root_unavailable)?;
        let cwd = absolute_project_root(PathBuf::from(project.root_path))?;
        return if cwd.is_dir() {
            Ok(cwd)
        } else {
            Err(task_project_root_unavailable())
        };
    }

    let worktree_id = task.worktree_id.ok_or_else(task_worktree_unavailable)?;
    let worktree = SqliteWorktreeRepository::new(pool.clone())
        .find_worktree(&worktree_id)
        .map_err(|_| task_worktree_unavailable())?
        .ok_or_else(task_worktree_unavailable)?;
    if worktree.task_id != task.id || worktree.activity != WorktreeActivity::Active {
        return Err(task_worktree_unavailable());
    }
    let branch_name = worktree.branch_name.ok_or_else(task_worktree_unavailable)?;
    let project = SqliteProjectRepository::new(pool.clone())
        .find_project(&task.project_id)
        .map_err(|_| task_worktree_unavailable())?
        .ok_or_else(task_worktree_unavailable)?;
    let repository = Repository::new(RepoRoot::new(project.root_path));
    let resolved = Git::new(CliGitRunner)
        .resolve_worktree_by_branch(ResolveWorktreeByBranchRequest {
            repository: &repository,
            branch_name: &branch_name,
        })
        .map_err(|_| task_worktree_unavailable())?;
    let cwd = resolved.worktree_root().as_path().to_path_buf();
    if !cwd.is_dir() {
        return Err(task_worktree_unavailable());
    }
    Ok(cwd)
}

/// Normalizes a stored project root before it crosses the ACP process boundary.
///
/// Relative project roots remain valid in persisted server configurations, while providers
/// require a stable absolute working directory after Ora starts them.
fn absolute_project_root(path: PathBuf) -> Result<PathBuf, BackendError> {
    if path.is_absolute() {
        return Ok(path);
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .map_err(|_| task_project_root_unavailable())
}

/// Builds the conflict used when task ownership cannot resolve an active Git worktree.
fn task_worktree_unavailable() -> BackendError {
    BackendError::new(
        BackendErrorKind::Conflict,
        "task_worktree_unavailable",
        "task worktree is unavailable",
    )
}

/// Builds the conflict used when a project-root task no longer has a usable directory.
fn task_project_root_unavailable() -> BackendError {
    BackendError::new(
        BackendErrorKind::Conflict,
        "task_project_root_unavailable",
        "task project root is unavailable",
    )
}

#[cfg(test)]
mod tests {
    use super::{absolute_project_root, resolve_task_cwd};
    use ora_application::{ProjectRepository, TaskRepository};
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
            resolve_task_cwd(&pool, &TaskId::new("task-1")).expect("resolve project root cwd"),
            project_root,
        );
    }

    /// Verifies relative roots are made stable before being passed to provider processes.
    #[test]
    fn normalizes_relative_project_roots_for_acp() {
        let cwd = absolute_project_root(PathBuf::from(".")).expect("resolve relative project root");
        assert!(cwd.is_absolute());
        assert!(cwd.is_dir());
    }
}
