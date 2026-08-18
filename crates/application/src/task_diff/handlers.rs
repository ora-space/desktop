use super::anchor::anchor_exists;
use super::mapper::{map_contract_diff_side, map_contract_thread_status, map_task_diff_comment};
use super::ports::{
    CommitTaskGitRequest, PushTaskGitRequest, ReadTaskDiffRequest, ReadTaskDiffScope,
    TaskDiffCommentIdGenerator, TaskDiffCommentRepository, TaskDiffReader, TaskGitWriter,
};
use crate::{ApplicationError, Clock, TaskRepository, WorktreeRepository};
use ora_contracts::{
    CommitTaskChangesRequest, CommitTaskChangesResponse, CreateTaskDiffCommentRequest,
    CreateTaskDiffCommentResponse, ListTaskDiffCommentsRequest, ListTaskDiffCommentsResponse,
    PushTaskBranchRequest, PushTaskBranchResponse, ReplyTaskDiffCommentRequest,
    ReplyTaskDiffCommentResponse, SetTaskDiffCommentStatusRequest,
    SetTaskDiffCommentStatusResponse, TaskDiffScope,
};
use ora_domain::{
    AuditFields, Task, TaskDiffAnchor, TaskDiffComment, TaskDiffCommentId, TaskDiffCommentKind,
    TaskDiffThreadStatus, TaskId, Worktree,
};
use ora_utils::path::PortableRelativePath;
use std::path::PathBuf;

/// Lists persisted root discussions and replies for one visible task.
pub struct ListTaskDiffCommentsHandler<TaskRepositoryPort, CommentRepository> {
    task_repository: TaskRepositoryPort,
    comment_repository: CommentRepository,
}

impl<TaskRepositoryPort, CommentRepository>
    ListTaskDiffCommentsHandler<TaskRepositoryPort, CommentRepository>
{
    /// Builds the comment list handler from task and comment persistence ports.
    pub fn new(task_repository: TaskRepositoryPort, comment_repository: CommentRepository) -> Self {
        Self {
            task_repository,
            comment_repository,
        }
    }
}

impl<TaskRepositoryPort, CommentRepository>
    ListTaskDiffCommentsHandler<TaskRepositoryPort, CommentRepository>
where
    TaskRepositoryPort: TaskRepository,
    CommentRepository: TaskDiffCommentRepository,
{
    /// Returns every visible discussion message in stable persistence order.
    pub fn handle(
        &self,
        request: ListTaskDiffCommentsRequest,
    ) -> Result<ListTaskDiffCommentsResponse, ApplicationError> {
        let task_id = TaskId::new(request.task_id);
        ensure_task_exists(&self.task_repository, &task_id)?;
        let comments = self
            .comment_repository
            .list_comments(&task_id)
            .map_err(ApplicationError::from_task_diff_comment_repository_error)?;

        Ok(ListTaskDiffCommentsResponse {
            comments: comments.into_iter().map(map_task_diff_comment).collect(),
        })
    }
}

/// Creates one line-anchored root discussion in a task diff.
pub struct CreateTaskDiffCommentHandler<
    TaskRepositoryPort,
    WorktreeRepositoryPort,
    DiffReader,
    CommentRepository,
    CommentIdGenerator,
    ClockSource,
> {
    task_repository: TaskRepositoryPort,
    worktree_repository: WorktreeRepositoryPort,
    diff_reader: DiffReader,
    comment_repository: CommentRepository,
    comment_id_generator: CommentIdGenerator,
    clock: ClockSource,
    worktree_path: PathBuf,
}

impl<
    TaskRepositoryPort,
    WorktreeRepositoryPort,
    DiffReader,
    CommentRepository,
    CommentIdGenerator,
    ClockSource,
>
    CreateTaskDiffCommentHandler<
        TaskRepositoryPort,
        WorktreeRepositoryPort,
        DiffReader,
        CommentRepository,
        CommentIdGenerator,
        ClockSource,
    >
{
    /// Builds the root discussion handler from persistence, identity, and clock dependencies.
    pub fn new(
        task_repository: TaskRepositoryPort,
        worktree_repository: WorktreeRepositoryPort,
        diff_reader: DiffReader,
        comment_repository: CommentRepository,
        comment_id_generator: CommentIdGenerator,
        clock: ClockSource,
        worktree_path: PathBuf,
    ) -> Self {
        Self {
            task_repository,
            worktree_repository,
            diff_reader,
            comment_repository,
            comment_id_generator,
            clock,
            worktree_path,
        }
    }
}

impl<
    TaskRepositoryPort,
    WorktreeRepositoryPort,
    DiffReader,
    CommentRepository,
    CommentIdGenerator,
    ClockSource,
>
    CreateTaskDiffCommentHandler<
        TaskRepositoryPort,
        WorktreeRepositoryPort,
        DiffReader,
        CommentRepository,
        CommentIdGenerator,
        ClockSource,
    >
where
    TaskRepositoryPort: TaskRepository,
    WorktreeRepositoryPort: WorktreeRepository,
    DiffReader: TaskDiffReader,
    CommentRepository: TaskDiffCommentRepository,
    CommentIdGenerator: TaskDiffCommentIdGenerator,
    ClockSource: Clock,
{
    /// Validates and persists one root line discussion.
    pub fn handle(
        &self,
        mut request: CreateTaskDiffCommentRequest,
    ) -> Result<CreateTaskDiffCommentResponse, ApplicationError> {
        let task_id = TaskId::new(request.task_id);
        let (_task, worktree) =
            load_task_worktree(&self.task_repository, &self.worktree_repository, &task_id)?;
        let base_commit_id = recorded_baseline(&worktree)?;
        validate_comment_body(&request.body)?;
        request.anchor.path = validate_anchor(
            &request.anchor.diff_id,
            &request.anchor.path,
            request.anchor.start_line,
            request.anchor.end_line,
        )?;
        let snapshot = self
            .diff_reader
            .read_task_diff(ReadTaskDiffRequest {
                worktree_path: self.worktree_path.clone(),
                base_commit_id: base_commit_id.to_string(),
                scope: match request.scope {
                    TaskDiffScope::Branch => ReadTaskDiffScope::Branch,
                    TaskDiffScope::Unstaged => ReadTaskDiffScope::Unstaged,
                    TaskDiffScope::Staged => ReadTaskDiffScope::Staged,
                    TaskDiffScope::Committed => ReadTaskDiffScope::Committed,
                },
            })
            .map_err(ApplicationError::from_task_diff_reader_error)?;
        let current_diff_id =
            task_diff_id(base_commit_id, &snapshot.head_commit_id, &snapshot.patch);
        if request.anchor.diff_id != current_diff_id {
            return Err(ApplicationError::TaskDiffStale);
        }
        if !anchor_exists(&snapshot.patch, &request.anchor) {
            return Err(ApplicationError::TaskDiffCommentInvalid {
                message: "comment anchor does not exist in the current diff".to_string(),
            });
        }
        let now = self.clock.now_timestamp_millis();
        let comment = TaskDiffComment::new(
            self.comment_id_generator.generate_comment_id(),
            task_id,
            TaskDiffCommentKind::Thread {
                anchor: TaskDiffAnchor {
                    diff_id: request.anchor.diff_id,
                    path: request.anchor.path,
                    side: map_contract_diff_side(request.anchor.side),
                    start_line: request.anchor.start_line,
                    end_line: request.anchor.end_line,
                    hunk_header: request.anchor.hunk_header,
                    line_content: request.anchor.line_content,
                },
                status: TaskDiffThreadStatus::Open,
            },
            request.body,
            AuditFields::new(now, now, false),
        );
        let comment = self
            .comment_repository
            .create_comment(comment)
            .map_err(ApplicationError::from_task_diff_comment_repository_error)?;

        Ok(CreateTaskDiffCommentResponse {
            comment: map_task_diff_comment(comment),
        })
    }
}

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

/// Returns the immutable baseline while keeping historical unavailable state explicit.
fn recorded_baseline(worktree: &Worktree) -> Result<&str, ApplicationError> {
    worktree
        .baseline
        .commit_id()
        .ok_or(ApplicationError::TaskDiffBaselineUnavailable)
}

/// Creates one reply beneath an existing task diff discussion message.
pub struct ReplyTaskDiffCommentHandler<
    TaskRepositoryPort,
    CommentRepository,
    CommentIdGenerator,
    ClockSource,
> {
    task_repository: TaskRepositoryPort,
    comment_repository: CommentRepository,
    comment_id_generator: CommentIdGenerator,
    clock: ClockSource,
}

impl<TaskRepositoryPort, CommentRepository, CommentIdGenerator, ClockSource>
    ReplyTaskDiffCommentHandler<
        TaskRepositoryPort,
        CommentRepository,
        CommentIdGenerator,
        ClockSource,
    >
{
    /// Builds the reply handler from persistence, identity, and clock dependencies.
    pub fn new(
        task_repository: TaskRepositoryPort,
        comment_repository: CommentRepository,
        comment_id_generator: CommentIdGenerator,
        clock: ClockSource,
    ) -> Self {
        Self {
            task_repository,
            comment_repository,
            comment_id_generator,
            clock,
        }
    }
}

impl<TaskRepositoryPort, CommentRepository, CommentIdGenerator, ClockSource>
    ReplyTaskDiffCommentHandler<
        TaskRepositoryPort,
        CommentRepository,
        CommentIdGenerator,
        ClockSource,
    >
where
    TaskRepositoryPort: TaskRepository,
    CommentRepository: TaskDiffCommentRepository,
    CommentIdGenerator: TaskDiffCommentIdGenerator,
    ClockSource: Clock,
{
    /// Validates the parent belongs to the task before persisting a reply.
    pub fn handle(
        &self,
        request: ReplyTaskDiffCommentRequest,
    ) -> Result<ReplyTaskDiffCommentResponse, ApplicationError> {
        let task_id = TaskId::new(request.task_id);
        ensure_task_exists(&self.task_repository, &task_id)?;
        validate_comment_body(&request.body)?;
        let parent_comment_id = TaskDiffCommentId::new(request.comment_id);
        let parent = load_comment(&self.comment_repository, &parent_comment_id)?;
        if parent.task_id != task_id {
            return Err(ApplicationError::TaskDiffCommentNotFound {
                comment_id: parent_comment_id.to_string(),
            });
        }
        if matches!(parent.kind, TaskDiffCommentKind::Reply { .. }) {
            return Err(ApplicationError::TaskDiffCommentInvalid {
                message: "replies must belong directly to a root discussion".to_string(),
            });
        }
        let now = self.clock.now_timestamp_millis();
        let comment = TaskDiffComment::new(
            self.comment_id_generator.generate_comment_id(),
            task_id,
            TaskDiffCommentKind::Reply { parent_comment_id },
            request.body,
            AuditFields::new(now, now, false),
        );
        let comment = self
            .comment_repository
            .create_comment(comment)
            .map_err(ApplicationError::from_task_diff_comment_repository_error)?;

        Ok(ReplyTaskDiffCommentResponse {
            comment: map_task_diff_comment(comment),
        })
    }
}

/// Changes the open/resolved state of one root diff discussion.
pub struct SetTaskDiffCommentStatusHandler<TaskRepositoryPort, CommentRepository, ClockSource> {
    task_repository: TaskRepositoryPort,
    comment_repository: CommentRepository,
    clock: ClockSource,
}

impl<TaskRepositoryPort, CommentRepository, ClockSource>
    SetTaskDiffCommentStatusHandler<TaskRepositoryPort, CommentRepository, ClockSource>
{
    /// Builds the discussion status handler from persistence and clock dependencies.
    pub fn new(
        task_repository: TaskRepositoryPort,
        comment_repository: CommentRepository,
        clock: ClockSource,
    ) -> Self {
        Self {
            task_repository,
            comment_repository,
            clock,
        }
    }
}

impl<TaskRepositoryPort, CommentRepository, ClockSource>
    SetTaskDiffCommentStatusHandler<TaskRepositoryPort, CommentRepository, ClockSource>
where
    TaskRepositoryPort: TaskRepository,
    CommentRepository: TaskDiffCommentRepository,
    ClockSource: Clock,
{
    /// Updates only root discussions because replies do not own resolution state.
    pub fn handle(
        &self,
        request: SetTaskDiffCommentStatusRequest,
    ) -> Result<SetTaskDiffCommentStatusResponse, ApplicationError> {
        let task_id = TaskId::new(request.task_id);
        ensure_task_exists(&self.task_repository, &task_id)?;
        let comment_id = TaskDiffCommentId::new(request.comment_id);
        let mut comment = load_comment(&self.comment_repository, &comment_id)?;
        if comment.task_id != task_id {
            return Err(ApplicationError::TaskDiffCommentNotFound {
                comment_id: comment_id.to_string(),
            });
        }

        match &mut comment.kind {
            TaskDiffCommentKind::Thread { status, .. } => {
                *status = map_contract_thread_status(request.status);
            }
            TaskDiffCommentKind::Reply { .. } => {
                return Err(ApplicationError::TaskDiffCommentInvalid {
                    message: "reply comments do not own resolution state".to_string(),
                });
            }
        }
        comment
            .audit_fields
            .touch(self.clock.now_timestamp_millis());
        let comment = self
            .comment_repository
            .update_comment(comment)
            .map_err(ApplicationError::from_task_diff_comment_repository_error)?;

        Ok(SetTaskDiffCommentStatusResponse {
            comment: map_task_diff_comment(comment),
        })
    }
}

/// Loads the task and its owned worktree while enforcing their persisted relationship.
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
    let worktree_id = task.worktree_id.as_ref().ok_or_else(|| {
        ApplicationError::task_diff_failure(std::io::Error::other("task does not own a worktree"))
    })?;
    let worktree = worktree_repository
        .find_worktree(worktree_id)
        .map_err(ApplicationError::from_worktree_repository_error)?
        .ok_or_else(|| ApplicationError::WorktreeNotFound {
            worktree_id: worktree_id.to_string(),
        })?;
    if worktree.task_id != *task_id {
        return Err(ApplicationError::task_diff_failure(std::io::Error::other(
            "task worktree ownership does not match persisted task",
        )));
    }

    Ok((task, worktree))
}

/// Loads one visible task so every comment operation shares identical not-found behavior.
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

/// Loads one visible comment and maps absence into the stable application error.
fn load_comment<CommentRepository>(
    comment_repository: &CommentRepository,
    comment_id: &TaskDiffCommentId,
) -> Result<TaskDiffComment, ApplicationError>
where
    CommentRepository: TaskDiffCommentRepository,
{
    comment_repository
        .find_comment(comment_id)
        .map_err(ApplicationError::from_task_diff_comment_repository_error)?
        .ok_or_else(|| ApplicationError::TaskDiffCommentNotFound {
            comment_id: comment_id.to_string(),
        })
}

/// Rejects blank discussion messages before they reach persistence.
pub(super) fn validate_comment_body(body: &str) -> Result<(), ApplicationError> {
    if body.trim().is_empty() {
        return Err(ApplicationError::TaskDiffCommentInvalid {
            message: "comment body must not be blank".to_string(),
        });
    }

    Ok(())
}

/// Validates stable revision metadata, line ranges, and worktree-relative file paths.
pub(super) fn validate_anchor(
    diff_id: &str,
    path: &str,
    start_line: u32,
    end_line: u32,
) -> Result<String, ApplicationError> {
    let path = PortableRelativePath::parse(path).map_err(|_| {
        ApplicationError::TaskDiffCommentInvalid {
            message: "comment anchor is invalid".to_string(),
        }
    })?;
    if diff_id.trim().is_empty() || path.is_root() || start_line == 0 || end_line < start_line {
        return Err(ApplicationError::TaskDiffCommentInvalid {
            message: "comment anchor is invalid".to_string(),
        });
    }

    Ok(path.as_str().to_string())
}

/// Produces a deterministic identifier with explicit field boundaries for stale snapshots.
pub fn task_diff_id(base_commit_id: &str, head_commit_id: &str, patch: &str) -> String {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x00000100000001b3;
    let mut hash = FNV_OFFSET_BASIS;
    for field in [
        base_commit_id.as_bytes(),
        head_commit_id.as_bytes(),
        patch.as_bytes(),
    ] {
        for byte in field.len().to_be_bytes().iter().chain(field) {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    }
    format!("fnv1a64:{hash:016x}")
}
