use super::ports::{CommitTaskGitRequest, PushTaskGitRequest, TaskGitWriter};
use crate::{ApplicationError, TaskRepository, WorktreeRepository};
use ora_contracts::{
    CommitTaskChangesRequest, CommitTaskChangesResponse, PushTaskBranchRequest,
    PushTaskBranchResponse,
};
use ora_domain::{Task, TaskId, Worktree};
use std::path::PathBuf;

/// Commits the complete change set from one persisted task worktree.
pub struct CommitTaskChangesHandler<TaskRepositoryPort, WorktreeRepositoryPort, GitWriter> {
    task_repository: TaskRepositoryPort,
    worktree_repository: WorktreeRepositoryPort,
    git_writer: GitWriter,
    worktree_path: PathBuf,
}

impl<TaskRepositoryPort, WorktreeRepositoryPort, GitWriter>
    CommitTaskChangesHandler<TaskRepositoryPort, WorktreeRepositoryPort, GitWriter>
{
    /// Builds a commit handler from persistence, Git, and backend path dependencies.
    pub fn new(
        task_repository: TaskRepositoryPort,
        worktree_repository: WorktreeRepositoryPort,
        git_writer: GitWriter,
        worktree_path: PathBuf,
    ) -> Self {
        Self {
            task_repository,
            worktree_repository,
            git_writer,
            worktree_path,
        }
    }
}

impl<TaskRepositoryPort, WorktreeRepositoryPort, GitWriter>
    CommitTaskChangesHandler<TaskRepositoryPort, WorktreeRepositoryPort, GitWriter>
where
    TaskRepositoryPort: TaskRepository,
    WorktreeRepositoryPort: WorktreeRepository,
    GitWriter: TaskGitWriter,
{
    /// Validates the message and commits only the task's verified branch.
    pub fn handle(
        &self,
        request: CommitTaskChangesRequest,
    ) -> Result<CommitTaskChangesResponse, ApplicationError> {
        let message = request.message.trim();
        if message.is_empty() {
            return Err(ApplicationError::TaskDiffCommitMessageBlank);
        }
        let task_id = TaskId::new(request.task_id);
        let (_task, worktree) =
            load_task_worktree(&self.task_repository, &self.worktree_repository, &task_id)?;
        let branch_name = recorded_branch(&worktree)?;
        let commit = self
            .git_writer
            .commit_changes(CommitTaskGitRequest {
                worktree_path: self.worktree_path.clone(),
                expected_branch_name: branch_name.to_string(),
                message: message.to_string(),
            })
            .map_err(task_git_writer_error)?;

        Ok(CommitTaskChangesResponse {
            commit_id: commit.commit_id,
            summary: commit.summary,
        })
    }
}

/// Pushes one persisted task worktree branch to its default remote.
pub struct PushTaskBranchHandler<TaskRepositoryPort, WorktreeRepositoryPort, GitWriter> {
    task_repository: TaskRepositoryPort,
    worktree_repository: WorktreeRepositoryPort,
    git_writer: GitWriter,
    worktree_path: PathBuf,
}

impl<TaskRepositoryPort, WorktreeRepositoryPort, GitWriter>
    PushTaskBranchHandler<TaskRepositoryPort, WorktreeRepositoryPort, GitWriter>
{
    /// Builds a push handler from persistence, Git, and backend path dependencies.
    pub fn new(
        task_repository: TaskRepositoryPort,
        worktree_repository: WorktreeRepositoryPort,
        git_writer: GitWriter,
        worktree_path: PathBuf,
    ) -> Self {
        Self {
            task_repository,
            worktree_repository,
            git_writer,
            worktree_path,
        }
    }
}

impl<TaskRepositoryPort, WorktreeRepositoryPort, GitWriter>
    PushTaskBranchHandler<TaskRepositoryPort, WorktreeRepositoryPort, GitWriter>
where
    TaskRepositoryPort: TaskRepository,
    WorktreeRepositoryPort: WorktreeRepository,
    GitWriter: TaskGitWriter,
{
    /// Pushes only after task ownership and the persisted branch are verified.
    pub fn handle(
        &self,
        request: PushTaskBranchRequest,
    ) -> Result<PushTaskBranchResponse, ApplicationError> {
        let task_id = TaskId::new(request.task_id);
        let (_task, worktree) =
            load_task_worktree(&self.task_repository, &self.worktree_repository, &task_id)?;
        let branch_name = recorded_branch(&worktree)?;
        let push = self
            .git_writer
            .push_branch(PushTaskGitRequest {
                worktree_path: self.worktree_path.clone(),
                expected_branch_name: branch_name.to_string(),
            })
            .map_err(task_git_writer_error)?;

        Ok(PushTaskBranchResponse {
            branch_name: push.branch_name,
            remote_name: push.remote_name,
        })
    }
}

/// Returns the persisted branch identity required by mutating Git operations.
fn recorded_branch(worktree: &Worktree) -> Result<&str, ApplicationError> {
    worktree.branch_name.as_deref().ok_or_else(|| {
        ApplicationError::task_diff_failure(std::io::Error::other(
            "task worktree has no recorded branch",
        ))
    })
}

/// Converts writer failures into the stable task Git error surface.
fn task_git_writer_error(error: super::TaskGitWriterError) -> ApplicationError {
    match error {
        super::TaskGitWriterError::OperationFailed(source) => ApplicationError::TaskDiff { source },
    }
}

/// Loads the task-owned worktree so commit and push share identical not-found behavior.
fn load_task_worktree<TaskRepositoryPort, WorktreeRepositoryPort>(
    task_repository: &TaskRepositoryPort,
    worktree_repository: &WorktreeRepositoryPort,
    task_id: &TaskId,
) -> Result<(Task, Worktree), ApplicationError>
where
    TaskRepositoryPort: TaskRepository,
    WorktreeRepositoryPort: WorktreeRepository,
{
    let task = ensure_task_exists(task_repository, task_id)?;
    let worktree = worktree_repository
        .find_worktree(&task.workspace_id)
        .map_err(ApplicationError::from_worktree_repository_error)?
        .ok_or_else(|| ApplicationError::WorktreeNotFound {
            workspace_id: task.workspace_id.to_string(),
        })?;

    Ok((task, worktree))
}

/// Loads one visible task so Git write operations share identical not-found behavior.
fn ensure_task_exists<TaskRepositoryPort>(
    task_repository: &TaskRepositoryPort,
    task_id: &TaskId,
) -> Result<Task, ApplicationError>
where
    TaskRepositoryPort: TaskRepository,
{
    task_repository
        .find_task(task_id)
        .map_err(ApplicationError::from_task_repository_error)?
        .ok_or_else(|| ApplicationError::TaskNotFound {
            task_id: task_id.to_string(),
        })
}
