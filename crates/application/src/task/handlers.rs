use crate::task::mapper::map_task;
use crate::task::ports::{
    CreateTaskWorktreeRequest, DeleteTaskWorktreeRequest, TaskIdGenerator, TaskRepository,
    TaskWorktreeDeletionMode, TaskWorktreeProvisioner,
};
use crate::worktree::{WorktreeIdGenerator, WorktreeRepository};
use crate::{ApplicationError, Clock};
use ora_contracts::{
    CreateTaskRequest, CreateTaskResponse, GetTaskRequest, GetTaskResponse, ListTasksRequest,
    ListTasksResponse, TaskStatus, TaskWorkspaceMode, UpdateTaskRequest, UpdateTaskResponse,
};
use ora_domain::{
    AuditFields, ProjectId, Task as DomainTask, TaskId, TaskStatus as DomainTaskStatus,
    Worktree as DomainWorktree, WorktreeActivity as DomainWorktreeActivity, WorktreeId,
};
use ora_logging::{ora_error, ora_info};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const TASK_BRANCH_PREFIX_LEN: usize = 8;
const MAX_TASK_ID_GENERATION_ATTEMPTS: usize = 3;

/// Handles task creation without depending on transport-specific concerns.
pub struct CreateTaskHandler<
    TaskRepositoryPort,
    WorktreeRepositoryPort,
    TaskIdGeneratorPort,
    WorktreeIdGeneratorPort,
    WorktreeProvisioner,
    ClockSource,
> {
    task_repository: TaskRepositoryPort,
    worktree_repository: WorktreeRepositoryPort,
    task_id_generator: TaskIdGeneratorPort,
    worktree_id_generator: WorktreeIdGeneratorPort,
    worktree_provisioner: WorktreeProvisioner,
    work_dir: PathBuf,
    clock: ClockSource,
}

impl<
    TaskRepositoryPort,
    WorktreeRepositoryPort,
    TaskIdGeneratorPort,
    WorktreeIdGeneratorPort,
    WorktreeProvisioner,
    ClockSource,
>
    CreateTaskHandler<
        TaskRepositoryPort,
        WorktreeRepositoryPort,
        TaskIdGeneratorPort,
        WorktreeIdGeneratorPort,
        WorktreeProvisioner,
        ClockSource,
    >
{
    pub fn new(
        task_repository: TaskRepositoryPort,
        worktree_repository: WorktreeRepositoryPort,
        task_id_generator: TaskIdGeneratorPort,
        worktree_id_generator: WorktreeIdGeneratorPort,
        worktree_provisioner: WorktreeProvisioner,
        work_dir: PathBuf,
        clock: ClockSource,
    ) -> Self {
        Self {
            task_repository,
            worktree_repository,
            task_id_generator,
            worktree_id_generator,
            worktree_provisioner,
            work_dir,
            clock,
        }
    }
}

impl<
    TaskRepositoryPort,
    WorktreeRepositoryPort,
    TaskIdGeneratorPort,
    WorktreeIdGeneratorPort,
    WorktreeProvisioner,
    ClockSource,
>
    CreateTaskHandler<
        TaskRepositoryPort,
        WorktreeRepositoryPort,
        TaskIdGeneratorPort,
        WorktreeIdGeneratorPort,
        WorktreeProvisioner,
        ClockSource,
    >
where
    TaskRepositoryPort: TaskRepository,
    WorktreeRepositoryPort: WorktreeRepository,
    TaskIdGeneratorPort: TaskIdGenerator,
    WorktreeIdGeneratorPort: WorktreeIdGenerator,
    WorktreeProvisioner: TaskWorktreeProvisioner,
    ClockSource: Clock,
{
    /// Creates a task in either an owned linked worktree or the project root.
    pub fn handle(
        &self,
        request: CreateTaskRequest,
    ) -> Result<CreateTaskResponse, ApplicationError> {
        match request.workspace_mode.unwrap_or_default() {
            TaskWorkspaceMode::Worktree => self.create_worktree_task(request),
            TaskWorkspaceMode::ProjectRoot => self.create_project_root_task(request),
        }
    }

    /// Provisions a linked worktree before persisting the task that owns it.
    fn create_worktree_task(
        &self,
        request: CreateTaskRequest,
    ) -> Result<CreateTaskResponse, ApplicationError> {
        self.worktree_provisioner
            .validate_repository()
            .map_err(ApplicationError::from_task_worktree_provisioner_error)
            .inspect_err(|error| log_task_failure("create_task", None, error))?;
        let task_id = self
            .select_available_task_id()
            .inspect_err(|error| log_task_failure("create_task", None, error))?;
        let branch_name = branch_name_for_task(&task_id);
        let worktree_path = worktree_path_for_task(&self.work_dir, &task_id);
        let provisioned_worktree = self
            .worktree_provisioner
            .create_task_worktree(CreateTaskWorktreeRequest {
                branch_name: branch_name.clone(),
                worktree_path,
            })
            .map_err(|error| {
                let error = ApplicationError::from_task_worktree_provisioner_error(error);
                log_task_failure("create_task", Some(&task_id), &error);
                error
            })?;

        let now = self.clock.now_timestamp_millis();
        let worktree_id = self.worktree_id_generator.generate_worktree_id();
        let worktree = DomainWorktree::new(
            worktree_id,
            task_id.clone(),
            Some(branch_name.clone()),
            ora_domain::WorktreeBaseline::recorded(provisioned_worktree.base_commit_id).map_err(
                |error| ApplicationError::TaskWorktree {
                    message: error.to_string(),
                },
            )?,
            DomainWorktreeActivity::Active,
            AuditFields::new(now, now, false),
        );
        let worktree = match self.worktree_repository.create_worktree(worktree) {
            Ok(worktree) => worktree,
            Err(error) => {
                return Err(self.handle_create_failure_after_provisioning(
                    "create_task",
                    &task_id,
                    &branch_name,
                    ApplicationError::from_worktree_repository_error(error),
                ));
            }
        };
        let task = DomainTask::new(
            task_id.clone(),
            ProjectId::new(request.project_id),
            request.title,
            map_contract_task_status(request.status),
            Some(worktree.id.clone()),
            AuditFields::new(now, now, false),
        );
        let task = match self.task_repository.create_task(task) {
            Ok(task) => task,
            Err(error) => {
                return Err(self.handle_task_persistence_failure_after_worktree_create(
                    &task_id,
                    &worktree.id,
                    &branch_name,
                    ApplicationError::from_task_repository_error(error),
                ));
            }
        };

        log_task_success("create_task", Some(&task.id));

        Ok(CreateTaskResponse {
            task: map_task(task),
        })
    }

    /// Persists a task that will run directly in its owning project's root directory.
    fn create_project_root_task(
        &self,
        request: CreateTaskRequest,
    ) -> Result<CreateTaskResponse, ApplicationError> {
        let task_id = self.task_id_generator.generate_task_id();
        let now = self.clock.now_timestamp_millis();
        let task = DomainTask::new(
            task_id.clone(),
            ProjectId::new(request.project_id),
            request.title,
            map_contract_task_status(request.status),
            None,
            AuditFields::new(now, now, false),
        );
        let task = self.task_repository.create_task(task).map_err(|error| {
            let error = ApplicationError::from_task_repository_error(error);
            log_task_failure("create_task", Some(&task_id), &error);
            error
        })?;

        log_task_success("create_task", Some(&task.id));

        Ok(CreateTaskResponse {
            task: map_task(task),
        })
    }

    /// Generates a task id whose branch prefix does not collide with existing task worktree folders.
    fn select_available_task_id(&self) -> Result<TaskId, ApplicationError> {
        for _ in 0..MAX_TASK_ID_GENERATION_ATTEMPTS {
            let task_id = self.task_id_generator.generate_task_id();
            let branch_prefix = task_branch_prefix(&task_id);

            if task_branch_prefix_exists_in_work_dir(&self.work_dir, &branch_prefix)? {
                continue;
            }

            let branch_name = branch_name_for_task(&task_id);
            let branch_exists = self
                .worktree_provisioner
                .task_branch_exists(&branch_name)
                .map_err(ApplicationError::from_task_worktree_provisioner_error)?;
            if !branch_exists {
                return Ok(task_id);
            }
        }

        Err(ApplicationError::TaskWorktree {
            message: format!(
                "failed to generate a task branch prefix without collision after {MAX_TASK_ID_GENERATION_ATTEMPTS} attempts"
            ),
        })
    }

    /// Attempts compensating worktree cleanup after persistence fails and returns the stable application error.
    fn handle_create_failure_after_provisioning(
        &self,
        operation: &'static str,
        task_id: &TaskId,
        branch_name: &str,
        original_error: ApplicationError,
    ) -> ApplicationError {
        let cleanup_result =
            self.worktree_provisioner
                .delete_task_worktree(DeleteTaskWorktreeRequest {
                    branch_name: branch_name.to_string(),
                    mode: TaskWorktreeDeletionMode::Force,
                });

        match cleanup_result {
            Ok(()) => {
                log_task_failure(operation, Some(task_id), &original_error);
                original_error
            }
            Err(cleanup_error) => {
                let cleanup_error =
                    ApplicationError::from_task_worktree_provisioner_error(cleanup_error);
                log_task_failure(operation, Some(task_id), &cleanup_error);
                cleanup_error
            }
        }
    }

    /// Soft-deletes the persisted worktree row, then removes the created checkout, before returning a stable failure.
    fn handle_task_persistence_failure_after_worktree_create(
        &self,
        task_id: &TaskId,
        worktree_id: &WorktreeId,
        branch_name: &str,
        original_error: ApplicationError,
    ) -> ApplicationError {
        let worktree_cleanup = self
            .worktree_repository
            .soft_delete_worktree(worktree_id, self.clock.now_timestamp_millis())
            .map_err(ApplicationError::from_worktree_repository_error);
        let filesystem_cleanup = self.handle_create_failure_after_provisioning(
            "create_task",
            task_id,
            branch_name,
            original_error,
        );

        match worktree_cleanup {
            Ok(_) => filesystem_cleanup,
            Err(error) => {
                log_task_failure("create_task", Some(task_id), &error);
                error
            }
        }
    }
}

/// Handles one task lookup without depending on transport-specific concerns.
pub struct GetTaskHandler<Repository> {
    repository: Repository,
}

impl<Repository> GetTaskHandler<Repository> {
    pub fn new(repository: Repository) -> Self {
        Self { repository }
    }
}

impl<Repository> GetTaskHandler<Repository>
where
    Repository: TaskRepository,
{
    /// Loads one visible task or returns a stable not-found application error.
    pub fn handle(&self, request: GetTaskRequest) -> Result<GetTaskResponse, ApplicationError> {
        let task_id = TaskId::new(request.task_id);
        let task = self.repository.find_task(&task_id).map_err(|error| {
            let error = ApplicationError::from_task_repository_error(error);
            log_task_failure("get_task", Some(&task_id), &error);
            error
        })?;

        match task {
            Some(task) => {
                log_task_success("get_task", Some(&task_id));

                Ok(GetTaskResponse {
                    task: map_task(task),
                })
            }
            None => {
                let error = ApplicationError::TaskNotFound {
                    task_id: task_id.to_string(),
                };
                log_task_failure("get_task", Some(&task_id), &error);
                Err(error)
            }
        }
    }
}

/// Handles task listing without depending on transport-specific concerns.
pub struct ListTasksHandler<Repository> {
    repository: Repository,
}

impl<Repository> ListTasksHandler<Repository> {
    pub fn new(repository: Repository) -> Self {
        Self { repository }
    }
}

impl<Repository> ListTasksHandler<Repository>
where
    Repository: TaskRepository,
{
    /// Lists every visible task and maps each one into the shared contract view.
    pub fn handle(
        &self,
        _request: ListTasksRequest,
    ) -> Result<ListTasksResponse, ApplicationError> {
        let tasks = self.repository.list_tasks().map_err(|error| {
            let error = ApplicationError::from_task_repository_error(error);
            log_task_failure("list_tasks", None, &error);
            error
        })?;

        ora_info!(
            message = "listed tasks",
            operation = "list_tasks",
            task_count = tasks.len()
        );

        Ok(ListTasksResponse {
            tasks: tasks.into_iter().map(map_task).collect(),
        })
    }
}

/// Handles task updates without depending on transport-specific concerns.
pub struct UpdateTaskHandler<Repository, ClockSource> {
    repository: Repository,
    clock: ClockSource,
}

impl<Repository, ClockSource> UpdateTaskHandler<Repository, ClockSource> {
    pub fn new(repository: Repository, clock: ClockSource) -> Self {
        Self { repository, clock }
    }
}

impl<Repository, ClockSource> UpdateTaskHandler<Repository, ClockSource>
where
    Repository: TaskRepository,
    ClockSource: Clock,
{
    /// Replaces the public task fields while preserving persistence-managed audit state.
    pub fn handle(
        &self,
        request: UpdateTaskRequest,
    ) -> Result<UpdateTaskResponse, ApplicationError> {
        let task_id = TaskId::new(request.task_id);
        let existing_task = self.repository.find_task(&task_id).map_err(|error| {
            let error = ApplicationError::from_task_repository_error(error);
            log_task_failure("update_task", Some(&task_id), &error);
            error
        })?;

        let existing_task = match existing_task {
            Some(existing_task) => existing_task,
            None => {
                let error = ApplicationError::TaskNotFound {
                    task_id: task_id.to_string(),
                };
                log_task_failure("update_task", Some(&task_id), &error);
                return Err(error);
            }
        };

        let task = DomainTask::new(
            task_id.clone(),
            existing_task.project_id,
            request.title,
            map_contract_task_status(request.status),
            existing_task.worktree_id,
            AuditFields::new(
                existing_task.audit_fields.created_at,
                self.clock.now_timestamp_millis(),
                existing_task.audit_fields.is_deleted,
            ),
        );
        let task = self.repository.update_task(task).map_err(|error| {
            let error = ApplicationError::from_task_repository_error(error);
            log_task_failure("update_task", Some(&task_id), &error);
            error
        })?;

        log_task_success("update_task", Some(&task_id));

        Ok(UpdateTaskResponse {
            task: map_task(task),
        })
    }
}

/// Emits the shared informational event shape for successful task CRUD operations.
fn log_task_success(operation: &'static str, task_id: Option<&TaskId>) {
    match task_id {
        Some(task_id) => {
            ora_info!(
                message = "task operation completed",
                operation,
                task_id = task_id.to_string()
            );
        }
        None => {
            ora_info!(message = "task operation completed", operation);
        }
    }
}

/// Emits the shared error event shape for failed task CRUD operations.
fn log_task_failure(operation: &'static str, task_id: Option<&TaskId>, error: &ApplicationError) {
    match (task_id, error) {
        (Some(task_id), ApplicationError::TaskNotFound { .. }) => {
            ora_error!(
                message = "task operation failed",
                operation,
                task_id = task_id.to_string(),
                error.kind = "task_not_found",
                error.message = error.to_string()
            );
        }
        (Some(task_id), ApplicationError::TaskRepository { .. }) => {
            ora_error!(
                message = "task operation failed",
                operation,
                task_id = task_id.to_string(),
                error.kind = "task_repository",
                error.message = error.to_string()
            );
        }
        (Some(task_id), ApplicationError::TaskWorktree { .. }) => {
            ora_error!(
                message = "task operation failed",
                operation,
                task_id = task_id.to_string(),
                error.kind = "task_worktree",
                error.message = error.to_string()
            );
        }
        (Some(task_id), ApplicationError::WorktreeNotFound { .. }) => {
            ora_error!(
                message = "task operation failed",
                operation,
                task_id = task_id.to_string(),
                error.kind = "worktree_not_found",
                error.message = error.to_string()
            );
        }
        (Some(task_id), ApplicationError::WorktreeRepository { .. }) => {
            ora_error!(
                message = "task operation failed",
                operation,
                task_id = task_id.to_string(),
                error.kind = "worktree_repository",
                error.message = error.to_string()
            );
        }
        (None, ApplicationError::TaskRepository { .. }) => {
            ora_error!(
                message = "task operation failed",
                operation,
                error.kind = "task_repository",
                error.message = error.to_string()
            );
        }
        (None, ApplicationError::TaskNotFound { .. }) => {
            ora_error!(
                message = "task operation failed",
                operation,
                error.kind = "task_not_found",
                error.message = error.to_string()
            );
        }
        (None, ApplicationError::TaskWorktree { .. }) => {
            ora_error!(
                message = "task operation failed",
                operation,
                error.kind = "task_worktree",
                error.message = error.to_string()
            );
        }
        (None, ApplicationError::TaskWorktreeRequiresGitRepository) => {
            ora_error!(
                message = "task operation failed",
                operation,
                error.kind = "worktree_requires_git_repository",
                error.message = error.to_string()
            );
        }
        _ => {}
    }
}

/// Translates the transport-facing task status into the domain enum.
fn map_contract_task_status(status: TaskStatus) -> DomainTaskStatus {
    match status {
        TaskStatus::Todo => DomainTaskStatus::Todo,
        TaskStatus::Doing => DomainTaskStatus::Doing,
        TaskStatus::Done => DomainTaskStatus::Done,
    }
}

/// Derives the stable task branch name from the first eight characters of the generated task id.
fn branch_name_for_task(task_id: &TaskId) -> String {
    format!("ora/{}", task_branch_prefix(task_id))
}

/// Derives the short branch prefix used to keep task branch names readable.
fn task_branch_prefix(task_id: &TaskId) -> String {
    task_id
        .to_string()
        .chars()
        .take(TASK_BRANCH_PREFIX_LEN)
        .collect()
}

/// Derives the owned linked-worktree path from the configured worktree root and full task id.
fn worktree_path_for_task(work_dir: &Path, task_id: &TaskId) -> PathBuf {
    work_dir.join(task_id.to_string())
}

/// Checks existing task worktree folders before branch creation because branch names use short ids.
fn task_branch_prefix_exists_in_work_dir(
    work_dir: &Path,
    branch_prefix: &str,
) -> Result<bool, ApplicationError> {
    let entries = match fs::read_dir(work_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(_) => {
            return Err(ApplicationError::TaskWorktree {
                message: "failed to inspect task worktree directory".to_string(),
            });
        }
    };

    for entry in entries {
        let entry = entry.map_err(|_| ApplicationError::TaskWorktree {
            message: "failed to inspect task worktree directory".to_string(),
        })?;
        let file_type = entry
            .file_type()
            .map_err(|_| ApplicationError::TaskWorktree {
                message: "failed to inspect task worktree directory".to_string(),
            })?;

        if file_type.is_dir()
            && entry
                .file_name()
                .to_string_lossy()
                .starts_with(branch_prefix)
        {
            return Ok(true);
        }
    }

    Ok(false)
}

#[cfg(test)]
mod task_branch_prefix_tests {
    use super::task_branch_prefix_exists_in_work_dir;
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::PathBuf;

    /// Verifies an absent worktree root is treated as the first task creation, not an inspection failure.
    #[test]
    fn reports_no_collision_when_work_dir_does_not_exist() {
        let work_dir = unique_test_work_dir("missing");

        assert_eq!(
            task_branch_prefix_exists_in_work_dir(&work_dir, "12345678"),
            Ok(false)
        );
    }

    /// Verifies only directories with the requested prefix reserve a task branch prefix.
    #[test]
    fn detects_matching_directory_prefixes() {
        let work_dir = unique_test_work_dir("matching-directory");
        fs::create_dir_all(work_dir.join("12345678-existing"))
            .unwrap_or_else(|error| panic!("failed to create collision fixture: {error}"));

        assert_eq!(
            task_branch_prefix_exists_in_work_dir(&work_dir, "12345678"),
            Ok(true)
        );

        fs::remove_dir_all(&work_dir)
            .unwrap_or_else(|error| panic!("failed to remove collision fixture: {error}"));
    }

    /// Verifies ordinary files and unrelated directories do not reserve a task branch prefix.
    #[test]
    fn ignores_files_and_unrelated_directories() {
        let work_dir = unique_test_work_dir("unrelated-entries");
        fs::create_dir_all(work_dir.join("87654321-existing"))
            .unwrap_or_else(|error| panic!("failed to create unrelated directory: {error}"));
        fs::write(work_dir.join("12345678-file"), b"not a worktree")
            .unwrap_or_else(|error| panic!("failed to create ordinary file: {error}"));

        assert_eq!(
            task_branch_prefix_exists_in_work_dir(&work_dir, "12345678"),
            Ok(false)
        );

        fs::remove_dir_all(&work_dir)
            .unwrap_or_else(|error| panic!("failed to remove unrelated entries: {error}"));
    }

    /// Builds an isolated filesystem location for branch-prefix unit tests.
    fn unique_test_work_dir(name: &str) -> PathBuf {
        let work_dir = std::env::temp_dir().join(format!(
            "ora-application-prefix-unit-{name}-{}",
            std::process::id()
        ));
        if work_dir.exists() {
            fs::remove_dir_all(&work_dir)
                .unwrap_or_else(|error| panic!("failed to reset unit-test work dir: {error}"));
        }
        work_dir
    }
}
