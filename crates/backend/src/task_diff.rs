use crate::clock::SystemClock;
use crate::error::{BackendError, BackendErrorKind};
use crate::task::resolve_task_cwd;
use ora_application::{
    CommitTaskChangesHandler, CreateTaskDiffCommentHandler, GitTaskDiffReader, GitTaskGitWriter,
    ListTaskDiffCommentsHandler, ProjectRepository, PushTaskBranchHandler, ReadTaskDiffRequest,
    ReadTaskDiffScope, ReplyTaskDiffCommentHandler, SetTaskDiffCommentStatusHandler,
    TaskDiffReader, TaskDiffReaderError, TaskRepository, UuidTaskDiffCommentIdGenerator,
    WorktreeRepository, task_diff_id,
};
use ora_contracts::{
    CommitTaskChangesRequest, CommitTaskChangesResponse, CreateTaskDiffCommentRequest,
    CreateTaskDiffCommentResponse, GetTaskDiffRequest, GetTaskDiffResponse,
    ListTaskDiffCommentsRequest, ListTaskDiffCommentsResponse, PushTaskBranchRequest,
    PushTaskBranchResponse, ReplyTaskDiffCommentRequest, ReplyTaskDiffCommentResponse,
    SetTaskDiffCommentStatusRequest, SetTaskDiffCommentStatusResponse, TaskDiffScope,
};
use ora_db::{
    RepositoryPool, SqliteProjectRepository, SqliteTaskDiffCommentRepository, SqliteTaskRepository,
    SqliteWorktreeRepository,
};
use ora_domain::{Project, Task, TaskId, WorktreeActivity};
use std::path::{Path, PathBuf};

/// Owns task-scoped Git review operations shared by the Web and Desktop adapters.
pub(crate) struct TaskDiffApi {
    pool: RepositoryPool,
    clock: SystemClock,
}

impl TaskDiffApi {
    /// Builds the shared task diff API from durable repositories and the system clock.
    pub(crate) fn new(pool: RepositoryPool, clock: SystemClock) -> Self {
        Self { pool, clock }
    }

    /// Computes a diff from the exact directory used as the task's agent session cwd.
    pub(crate) fn get_diff(
        &self,
        request: GetTaskDiffRequest,
    ) -> Result<GetTaskDiffResponse, BackendError> {
        let task_id = TaskId::new(request.task_id);
        let task = self.load_task(&task_id)?;
        let project = self.load_project(&task)?;
        let cwd = resolve_task_cwd(&self.pool, &task_id)?;
        let fixed_baseline = self.fixed_worktree_baseline(&task)?;
        let requested_baseline = fixed_baseline.as_deref().unwrap_or("HEAD");
        let snapshot = GitTaskDiffReader::new(PathBuf::from(project.root_path))
            .read_task_diff(ReadTaskDiffRequest {
                worktree_path: cwd,
                base_commit_id: requested_baseline.to_string(),
                scope: map_diff_scope(request.scope),
            })
            .map_err(map_diff_reader_error)?;

        // Direct-chat tasks intentionally follow the main checkout. HEAD is resolved
        // per read so their review surface mirrors Codex's current working-tree view.
        let base_commit_id = fixed_baseline.unwrap_or_else(|| snapshot.head_commit_id.clone());
        let diff_id = task_diff_id(&base_commit_id, &snapshot.head_commit_id, &snapshot.patch);

        Ok(GetTaskDiffResponse {
            base_commit_id,
            head_commit_id: snapshot.head_commit_id,
            diff_id,
            patch: snapshot.patch,
        })
    }

    /// Commits current changes only for tasks that own an isolated worktree.
    pub(crate) fn commit_changes(
        &self,
        request: CommitTaskChangesRequest,
    ) -> Result<CommitTaskChangesResponse, BackendError> {
        let (task, project, work_dir) = self.worktree_context(&request.task_id)?;
        CommitTaskChangesHandler::new(
            SqliteTaskRepository::new(self.pool.clone()),
            SqliteWorktreeRepository::new(self.pool.clone()),
            GitTaskGitWriter::new(PathBuf::from(project.root_path)),
            work_dir,
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
        let (task, project, work_dir) = self.worktree_context(&request.task_id)?;
        PushTaskBranchHandler::new(
            SqliteTaskRepository::new(self.pool.clone()),
            SqliteWorktreeRepository::new(self.pool.clone()),
            GitTaskGitWriter::new(PathBuf::from(project.root_path)),
            work_dir,
        )
        .handle(PushTaskBranchRequest {
            task_id: task.id.to_string(),
        })
        .map_err(BackendError::from)
    }

    /// Lists every persisted discussion message for one visible task.
    pub(crate) fn list_comments(
        &self,
        request: ListTaskDiffCommentsRequest,
    ) -> Result<ListTaskDiffCommentsResponse, BackendError> {
        ListTaskDiffCommentsHandler::new(
            SqliteTaskRepository::new(self.pool.clone()),
            SqliteTaskDiffCommentRepository::new(self.pool.clone()),
        )
        .handle(request)
        .map_err(BackendError::from)
    }

    /// Creates an anchored discussion for an isolated task worktree snapshot.
    pub(crate) fn create_comment(
        &self,
        request: CreateTaskDiffCommentRequest,
    ) -> Result<CreateTaskDiffCommentResponse, BackendError> {
        let (task, project, work_dir) = self.worktree_context(&request.task_id)?;
        CreateTaskDiffCommentHandler::new(
            SqliteTaskRepository::new(self.pool.clone()),
            SqliteWorktreeRepository::new(self.pool.clone()),
            GitTaskDiffReader::new(PathBuf::from(project.root_path)),
            SqliteTaskDiffCommentRepository::new(self.pool.clone()),
            UuidTaskDiffCommentIdGenerator::new(),
            self.clock,
            work_dir,
        )
        .handle(CreateTaskDiffCommentRequest {
            task_id: task.id.to_string(),
            anchor: request.anchor,
            body: request.body,
        })
        .map_err(BackendError::from)
    }

    /// Adds one reply beneath an existing task diff discussion.
    pub(crate) fn reply_comment(
        &self,
        request: ReplyTaskDiffCommentRequest,
    ) -> Result<ReplyTaskDiffCommentResponse, BackendError> {
        ReplyTaskDiffCommentHandler::new(
            SqliteTaskRepository::new(self.pool.clone()),
            SqliteTaskDiffCommentRepository::new(self.pool.clone()),
            UuidTaskDiffCommentIdGenerator::new(),
            self.clock,
        )
        .handle(request)
        .map_err(BackendError::from)
    }

    /// Resolves or reopens one root task diff discussion.
    pub(crate) fn set_comment_status(
        &self,
        request: SetTaskDiffCommentStatusRequest,
    ) -> Result<SetTaskDiffCommentStatusResponse, BackendError> {
        SetTaskDiffCommentStatusHandler::new(
            SqliteTaskRepository::new(self.pool.clone()),
            SqliteTaskDiffCommentRepository::new(self.pool.clone()),
            self.clock,
        )
        .handle(request)
        .map_err(BackendError::from)
    }

    /// Loads one visible task while keeping storage diagnostics behind the backend contract.
    fn load_task(&self, task_id: &TaskId) -> Result<Task, BackendError> {
        SqliteTaskRepository::new(self.pool.clone())
            .find_task(task_id)
            .map_err(|_| task_diff_internal())?
            .ok_or_else(|| {
                BackendError::new(
                    BackendErrorKind::NotFound,
                    "task_not_found",
                    format!("task not found: {task_id}"),
                )
            })
    }

    /// Loads the project that owns a task so Git operations target the correct repository.
    fn load_project(&self, task: &Task) -> Result<Project, BackendError> {
        SqliteProjectRepository::new(self.pool.clone())
            .find_project(&task.project_id)
            .map_err(|_| task_diff_internal())?
            .ok_or_else(|| {
                BackendError::new(
                    BackendErrorKind::NotFound,
                    "project_not_found",
                    format!("project not found: {}", task.project_id),
                )
            })
    }

    /// Returns the immutable worktree baseline, or no baseline for a project-root task.
    fn fixed_worktree_baseline(&self, task: &Task) -> Result<Option<String>, BackendError> {
        let Some(worktree_id) = task.worktree_id.as_ref() else {
            return Ok(None);
        };
        let worktree = SqliteWorktreeRepository::new(self.pool.clone())
            .find_worktree(worktree_id)
            .map_err(|_| task_diff_internal())?
            .ok_or_else(|| {
                BackendError::new(
                    BackendErrorKind::NotFound,
                    "worktree_not_found",
                    format!("worktree not found: {worktree_id}"),
                )
            })?;
        if worktree.task_id != task.id || worktree.activity != WorktreeActivity::Active {
            return Err(BackendError::new(
                BackendErrorKind::Conflict,
                "task_worktree_unavailable",
                "task worktree is unavailable",
            ));
        }
        worktree
            .baseline
            .commit_id()
            .map(str::to_string)
            .map(Some)
            .ok_or_else(|| {
                BackendError::new(
                    BackendErrorKind::Conflict,
                    "task_diff_baseline_unavailable",
                    "task diff baseline is unavailable",
                )
            })
    }

    /// Resolves the repository and parent directory required by worktree-only write operations.
    fn worktree_context(&self, task_id: &str) -> Result<(Task, Project, PathBuf), BackendError> {
        let task_id = TaskId::new(task_id);
        let task = self.load_task(&task_id)?;
        if task.worktree_id.is_none() {
            return Err(BackendError::new(
                BackendErrorKind::Conflict,
                "task_worktree_required",
                "this operation requires an isolated task worktree",
            ));
        }
        let project = self.load_project(&task)?;
        let cwd = resolve_task_cwd(&self.pool, &task_id)?;
        let work_dir = cwd
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(task_diff_internal)?;
        Ok((task, project, work_dir))
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
            BackendErrorKind::Internal,
            "task_diff_too_large",
            "task diff exceeds the response limit",
        ),
        TaskDiffReaderError::OperationFailed(_) => task_diff_internal(),
    }
}

/// Builds the sanitized failure shared by repository and Git implementation errors.
fn task_diff_internal() -> BackendError {
    BackendError::new(
        BackendErrorKind::Internal,
        "task_diff_error",
        "task diff operation failed",
    )
}

#[cfg(test)]
mod tests {
    use crate::{Backend, BackendPaths};
    use ora_contracts::{
        CreateProjectRequest, CreateTaskRequest, GetTaskDiffRequest, TaskDiffScope, TaskStatus,
        TaskWorkspaceMode,
    };
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use tempfile::TempDir;

    /// Verifies direct-chat edits are read from the same project root used by the agent.
    #[test]
    fn captures_agent_changes_from_project_root_tasks() {
        let temporary = TempDir::new().expect("create temporary backend directory");
        let repository_root = temporary.path().join("repository");
        initialize_repository(&repository_root);
        let backend = open_backend(&temporary);
        let project_id = create_project(&backend, &repository_root);
        let task = backend
            .create_task(CreateTaskRequest {
                project_id,
                title: "Direct chat".to_string(),
                status: TaskStatus::Todo,
                workspace_mode: Some(TaskWorkspaceMode::ProjectRoot),
            })
            .expect("create project-root task")
            .task;

        fs::write(
            repository_root.join("README.md"),
            "ora backend test\nchanged by agent\n",
        )
        .expect("write agent change");

        let response = backend
            .get_task_diff(GetTaskDiffRequest {
                task_id: task.id,
                scope: TaskDiffScope::Branch,
            })
            .expect("read project-root task diff");

        assert!(response.patch.contains("+changed by agent"));
        assert_eq!(response.base_commit_id, response.head_commit_id);
    }

    /// Verifies isolated task edits are read from the exact cwd resolved for the agent session.
    #[test]
    fn captures_agent_changes_from_worktree_tasks() {
        let temporary = TempDir::new().expect("create temporary backend directory");
        let repository_root = temporary.path().join("repository");
        initialize_repository(&repository_root);
        let backend = open_backend(&temporary);
        let project_id = create_project(&backend, &repository_root);
        let task = backend
            .create_task(CreateTaskRequest {
                project_id,
                title: "Isolated task".to_string(),
                status: TaskStatus::Todo,
                workspace_mode: Some(TaskWorkspaceMode::Worktree),
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
            worktree_root: temporary.path().join("worktrees"),
            home_directory: temporary.path().to_path_buf(),
        })
        .expect("open shared backend")
    }

    /// Persists a project pointing at the initialized repository fixture.
    fn create_project(backend: &Backend, repository_root: &Path) -> String {
        backend
            .create_project(CreateProjectRequest {
                name: "Ora".to_string(),
                root_path: repository_root.to_string_lossy().into_owned(),
            })
            .expect("create project")
            .project
            .id
    }

    /// Initializes a repository with one commit so both workspace modes can produce diffs.
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

    /// Runs one required Git setup command for the repository fixture.
    fn run_git(repository_root: &Path, arguments: &[&str]) {
        let status = Command::new("git")
            .current_dir(repository_root)
            .args(arguments)
            .status()
            .unwrap_or_else(|error| panic!("failed to start git {arguments:?}: {error}"));

        assert!(status.success(), "git {arguments:?} failed with {status}");
    }
}
