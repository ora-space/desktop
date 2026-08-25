use crate::error::{BackendError, ErrorClassification};
use crate::task::resolve_task_cwd;
use ora_application::{
    CommitTaskChangesHandler, GitTaskDiffReader, GitTaskGitWriter, PushTaskBranchHandler,
    ReadTaskDiffRequest, ReadTaskDiffScope, TaskDiffReader, TaskDiffReaderError, TaskRepository,
    WorktreeRepository,
};
use ora_contracts::{
    CommitTaskChangesRequest, CommitTaskChangesResponse, GetTaskDiffRequest, GetTaskDiffResponse,
    PushTaskBranchRequest, PushTaskBranchResponse, TaskDiffScope,
};
use ora_contracts::{EmptyErrorParams, PublicError};
use ora_db::{
    RepositoryPool, SqliteTaskRepository, SqliteWorkspaceRepository, SqliteWorktreeRepository,
};
use ora_domain::{Task, TaskId, WorkspaceLocation};
use std::path::PathBuf;

/// Owns task-scoped Git review operations shared by the Web and Desktop adapters.
pub(crate) struct TaskDiffApi {
    pool: RepositoryPool,
    git_cleanup: crate::git_cleanup::GitCleanupHandle,
    relative_path_base: PathBuf,
}

impl TaskDiffApi {
    /// Builds the shared task diff API from durable repositories and Git cleanup.
    pub(crate) fn new(
        pool: RepositoryPool,
        git_cleanup: crate::git_cleanup::GitCleanupHandle,
        relative_path_base: PathBuf,
    ) -> Self {
        Self {
            pool,
            git_cleanup,
            relative_path_base,
        }
    }

    /// Computes a diff from the exact directory used as the task's agent session cwd.
    pub(crate) fn get_diff(
        &self,
        request: GetTaskDiffRequest,
    ) -> Result<GetTaskDiffResponse, BackendError> {
        let task_id = TaskId::new(request.task_id);
        let task = self.load_task(&task_id)?;
        // Shared use lease: physical cleanup of this Workspace's checkout waits
        // for this read instead of removing the directory underneath it.
        let _worktree_use = self
            .git_cleanup
            .shared_worktree_use(task.workspace_id.as_ref());
        let repository_root = self.load_repository_root(&task)?;
        let cwd = resolve_task_cwd(&self.pool, &task_id, &self.relative_path_base)?;

        let worktree = SqliteWorktreeRepository::new(self.pool.clone())
            .find_worktree(&task.workspace_id)
            .map_err(task_diff_internal)?
            .ok_or_else(|| {
                BackendError::new(
                    ErrorClassification::NotFound,
                    PublicError::WorktreeNotFound(EmptyErrorParams {}),
                    "worktree not found",
                )
            })?;
        let base_commit_id = worktree.baseline.commit_id().ok_or_else(|| {
            BackendError::new(
                ErrorClassification::Conflict,
                PublicError::TaskDiffBaselineUnavailable(EmptyErrorParams {}),
                "task diff baseline is unavailable",
            )
        })?;
        let snapshot = GitTaskDiffReader::new(repository_root)
            .read_task_diff(ReadTaskDiffRequest {
                worktree_path: cwd,
                base_commit_id: base_commit_id.to_string(),
                scope: map_diff_scope(request.scope),
            })
            .map_err(map_diff_reader_error)?;

        Ok(GetTaskDiffResponse {
            base_commit_id: base_commit_id.to_string(),
            head_commit_id: snapshot.head_commit_id,
            patch: snapshot.patch,
        })
    }

    /// Commits current changes only for tasks that own an isolated worktree.
    pub(crate) fn commit_changes(
        &self,
        request: CommitTaskChangesRequest,
    ) -> Result<CommitTaskChangesResponse, BackendError> {
        let task_id = TaskId::new(request.task_id.clone());
        let task = self.load_task(&task_id)?;
        // Shared use lease: see get_diff; commits must not lose the checkout mid-write.
        let _worktree_use = self
            .git_cleanup
            .shared_worktree_use(task.workspace_id.as_ref());
        let (task, repository_root, worktree_path) = self.worktree_context(task)?;
        CommitTaskChangesHandler::new(
            SqliteTaskRepository::new(self.pool.clone()),
            SqliteWorktreeRepository::new(self.pool.clone()),
            GitTaskGitWriter::new(repository_root),
            worktree_path,
        )
        .handle(CommitTaskChangesRequest {
            task_id: task.id.to_string(),
            message: request.message,
        })
        .map_err(BackendError::from)
    }

    /// Pushes the verified branch owned by one isolated task worktree.
    pub(crate) fn push_branch(
        &self,
        request: PushTaskBranchRequest,
    ) -> Result<PushTaskBranchResponse, BackendError> {
        let task_id = TaskId::new(request.task_id);
        let task = self.load_task(&task_id)?;
        // Shared use lease: see get_diff; pushes read the checkout's branch state.
        let _worktree_use = self
            .git_cleanup
            .shared_worktree_use(task.workspace_id.as_ref());
        let (task, repository_root, worktree_path) = self.worktree_context(task)?;
        PushTaskBranchHandler::new(
            SqliteTaskRepository::new(self.pool.clone()),
            SqliteWorktreeRepository::new(self.pool.clone()),
            GitTaskGitWriter::new(repository_root),
            worktree_path,
        )
        .handle(PushTaskBranchRequest {
            task_id: task.id.to_string(),
        })
        .map_err(BackendError::from)
    }

    /// Loads one visible task while keeping storage diagnostics behind the backend contract.
    fn load_task(&self, task_id: &TaskId) -> Result<Task, BackendError> {
        SqliteTaskRepository::new(self.pool.clone())
            .find_task(task_id)
            .map_err(task_diff_internal)?
            .ok_or_else(|| {
                BackendError::new(
                    ErrorClassification::NotFound,
                    PublicError::TaskNotFound(EmptyErrorParams {}),
                    "task not found",
                )
            })
    }

    /// Resolves a task's repository from its project's main workspace location.
    fn load_repository_root(&self, task: &Task) -> Result<PathBuf, BackendError> {
        let workspace = SqliteWorkspaceRepository::new(self.pool.clone())
            .find_main_workspace(&task.project_id)
            .map_err(task_diff_internal)?
            .ok_or_else(|| {
                BackendError::new(
                    ErrorClassification::NotFound,
                    PublicError::ProjectNotFound(EmptyErrorParams {}),
                    "project not found",
                )
            })?;
        let WorkspaceLocation::LocalFilesystem { path } = workspace.location else {
            return Err(BackendError::new(
                ErrorClassification::Conflict,
                PublicError::WorkspaceUnavailable(EmptyErrorParams {}),
                "workspace is unavailable",
            ));
        };
        crate::task::absolute_project_root(PathBuf::from(path), &self.relative_path_base)
    }

    /// Resolves the repository and parent directory required by worktree-only write operations.
    fn worktree_context(&self, task: Task) -> Result<(Task, PathBuf, PathBuf), BackendError> {
        let task_id = task.id.clone();
        let repository_root = self.load_repository_root(&task)?;
        let cwd = resolve_task_cwd(&self.pool, &task_id, &self.relative_path_base)?;
        Ok((task, repository_root, cwd))
    }
}

/// Maps the public Git layer selector into the application diff reader vocabulary.
fn map_diff_scope(scope: TaskDiffScope) -> ReadTaskDiffScope {
    match scope {
        TaskDiffScope::Branch => ReadTaskDiffScope::Branch,
        TaskDiffScope::Unstaged => ReadTaskDiffScope::Unstaged,
        TaskDiffScope::Staged => ReadTaskDiffScope::Staged,
        TaskDiffScope::Committed => ReadTaskDiffScope::Committed,
    }
}

/// Preserves the public size limit while hiding Git execution diagnostics.
fn map_diff_reader_error(error: TaskDiffReaderError) -> BackendError {
    match error {
        TaskDiffReaderError::TooLarge { .. } => BackendError::new(
            ErrorClassification::PayloadTooLarge,
            PublicError::TaskDiffTooLarge(EmptyErrorParams {}),
            "task diff exceeds the response limit",
        ),
        TaskDiffReaderError::OperationFailed(source) => {
            BackendError::internal_boxed("task diff operation failed", source)
        }
    }
}

/// Retains repository diagnostics while exposing only the transport-neutral internal error.
fn task_diff_internal(source: impl std::error::Error + Send + Sync + 'static) -> BackendError {
    BackendError::internal("task diff operation failed", source)
}

#[cfg(test)]
mod tests {
    use crate::{Backend, BackendPaths};
    use ora_contracts::{
        CreateProjectRequest, CreateTaskRequest, GetTaskDiffRequest, TaskDiffScope,
    };
    use ora_test_support::GitTestScaffold;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    /// Verifies isolated task edits are read from the exact cwd resolved for the agent session.
    #[test]
    fn captures_agent_changes_from_worktree_tasks() {
        let temporary = TempDir::new().expect("create temporary backend directory");
        let scaffold =
            GitTestScaffold::new("backend-task-diff-worktree").expect("create Git test scaffold");
        scaffold
            .write_file(scaffold.repo_path(), "README.md", "ora backend test\n")
            .expect("write repository seed file");
        scaffold
            .stage_all_and_commit("initial")
            .expect("create repository seed commit");
        let repository_root = scaffold.repo_path();
        let backend = open_backend(&temporary);
        let project_id = create_project(&backend, &repository_root);
        let task = backend
            .create_task(CreateTaskRequest {
                project_id,
                title: "Isolated task".to_string(),
                base_branch: Some("main".to_string()),
            })
            .expect("create worktree task")
            .task;
        let agent_cwd = backend
            .resolve_task_cwd(&task.id)
            .expect("resolve agent cwd");

        fs::write(agent_cwd.join("agent-change.txt"), "captured\n")
            .expect("write agent worktree change");

        let response = backend
            .get_task_diff(GetTaskDiffRequest {
                task_id: task.id,
                scope: TaskDiffScope::Branch,
            })
            .expect("read worktree task diff");

        assert!(response.patch.contains("agent-change.txt"));
        assert!(response.patch.contains("+captured"));
    }

    /// Opens one isolated backend whose worktrees stay inside the test fixture.
    fn open_backend(temporary: &TempDir) -> Backend {
        Backend::open(BackendPaths {
            database_path: temporary.path().join("ora.sqlite3"),
            data_directory: temporary.path().to_path_buf(),
            deno_path: std::path::PathBuf::from("deno"),
            worktree_root: temporary.path().join("worktrees"),
            home_directory: temporary.path().to_path_buf(),
            relative_path_base: temporary.path().to_path_buf(),
            sessions_root: temporary.path().join("sessions"),
            skills_root: temporary.path().join("atoms").join("skills"),
            ripgrep_path: std::path::PathBuf::from("rg"),
            timezone: chrono_tz::UTC,
        })
        .expect("open shared backend")
    }

    /// Persists a project pointing at the initialized repository fixture.
    fn create_project(backend: &Backend, repository_root: &Path) -> String {
        backend
            .create_project(CreateProjectRequest {
                name: "Ora".to_string(),
                main_workspace_path: repository_root.to_string_lossy().into_owned(),
            })
            .expect("create project")
            .project
            .id
    }
}
