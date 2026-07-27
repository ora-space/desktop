use crate::bootstrap::SystemClock;
use ora_application::{
    ApplicationError, CommitTaskChangesHandler, CreateTaskDiffCommentHandler, GetTaskDiffHandler,
    GitTaskDiffReader, GitTaskGitWriter, ListTaskDiffCommentsHandler, PushTaskBranchHandler,
    ReplyTaskDiffCommentHandler, SetTaskDiffCommentStatusHandler, UuidTaskDiffCommentIdGenerator,
};
use ora_contracts::{
    CommitTaskChangesRequest, CommitTaskChangesResponse, CreateTaskDiffCommentRequest,
    CreateTaskDiffCommentResponse, GetTaskDiffRequest, GetTaskDiffResponse,
    ListTaskDiffCommentsRequest, ListTaskDiffCommentsResponse, PushTaskBranchRequest,
    PushTaskBranchResponse, ReplyTaskDiffCommentRequest, ReplyTaskDiffCommentResponse,
    SetTaskDiffCommentStatusRequest, SetTaskDiffCommentStatusResponse,
};
use ora_db::{
    RepositoryPool, SqliteTaskDiffCommentRepository, SqliteTaskRepository, SqliteWorktreeRepository,
};
use std::path::PathBuf;

type CreateCommentHandler = CreateTaskDiffCommentHandler<
    SqliteTaskRepository,
    SqliteWorktreeRepository,
    GitTaskDiffReader,
    SqliteTaskDiffCommentRepository,
    UuidTaskDiffCommentIdGenerator,
    SystemClock,
>;
type ReplyCommentHandler = ReplyTaskDiffCommentHandler<
    SqliteTaskRepository,
    SqliteTaskDiffCommentRepository,
    UuidTaskDiffCommentIdGenerator,
    SystemClock,
>;

/// Groups task diff and review-comment application handlers for the HTTP adapter.
pub struct TaskDiffApi {
    get_diff: GetTaskDiffHandler<SqliteTaskRepository, SqliteWorktreeRepository, GitTaskDiffReader>,
    list_comments:
        ListTaskDiffCommentsHandler<SqliteTaskRepository, SqliteTaskDiffCommentRepository>,
    create_comment: CreateCommentHandler,
    reply_comment: ReplyCommentHandler,
    set_comment_status: SetTaskDiffCommentStatusHandler<
        SqliteTaskRepository,
        SqliteTaskDiffCommentRepository,
        SystemClock,
    >,
    commit_changes:
        CommitTaskChangesHandler<SqliteTaskRepository, SqliteWorktreeRepository, GitTaskGitWriter>,
    push_branch:
        PushTaskBranchHandler<SqliteTaskRepository, SqliteWorktreeRepository, GitTaskGitWriter>,
}

impl TaskDiffApi {
    /// Builds the task diff API while keeping Git and persistence details outside HTTP handlers.
    pub fn new(
        pool: RepositoryPool,
        project_root: PathBuf,
        work_dir: PathBuf,
        clock: SystemClock,
    ) -> Self {
        let task_repository = SqliteTaskRepository::new(pool.clone());
        let worktree_repository = SqliteWorktreeRepository::new(pool.clone());
        let diff_reader = GitTaskDiffReader::new(project_root.clone());
        let git_writer = GitTaskGitWriter::new(project_root);
        let comment_repository = SqliteTaskDiffCommentRepository::new(pool);

        Self {
            get_diff: GetTaskDiffHandler::new(
                task_repository.clone(),
                worktree_repository.clone(),
                diff_reader.clone(),
                work_dir.clone(),
            ),
            list_comments: ListTaskDiffCommentsHandler::new(
                task_repository.clone(),
                comment_repository.clone(),
            ),
            create_comment: CreateTaskDiffCommentHandler::new(
                task_repository.clone(),
                worktree_repository.clone(),
                diff_reader,
                comment_repository.clone(),
                UuidTaskDiffCommentIdGenerator::new(),
                clock,
                work_dir.clone(),
            ),
            reply_comment: ReplyTaskDiffCommentHandler::new(
                task_repository.clone(),
                comment_repository.clone(),
                UuidTaskDiffCommentIdGenerator::new(),
                clock,
            ),
            set_comment_status: SetTaskDiffCommentStatusHandler::new(
                task_repository.clone(),
                comment_repository,
                clock,
            ),
            commit_changes: CommitTaskChangesHandler::new(
                task_repository.clone(),
                worktree_repository.clone(),
                git_writer.clone(),
                work_dir.clone(),
            ),
            push_branch: PushTaskBranchHandler::new(
                task_repository,
                worktree_repository,
                git_writer,
                work_dir,
            ),
        }
    }

    /// Returns one task's fixed-baseline unified diff.
    pub fn get_diff(
        &self,
        request: GetTaskDiffRequest,
    ) -> Result<GetTaskDiffResponse, ApplicationError> {
        self.get_diff.handle(request)
    }

    /// Commits all current changes from a task worktree.
    pub fn commit_changes(
        &self,
        request: CommitTaskChangesRequest,
    ) -> Result<CommitTaskChangesResponse, ApplicationError> {
        self.commit_changes.handle(request)
    }

    /// Pushes a task worktree's verified branch.
    pub fn push_branch(
        &self,
        request: PushTaskBranchRequest,
    ) -> Result<PushTaskBranchResponse, ApplicationError> {
        self.push_branch.handle(request)
    }

    /// Lists all persisted task diff discussions and replies.
    pub fn list_comments(
        &self,
        request: ListTaskDiffCommentsRequest,
    ) -> Result<ListTaskDiffCommentsResponse, ApplicationError> {
        self.list_comments.handle(request)
    }

    /// Creates one root line discussion.
    pub fn create_comment(
        &self,
        request: CreateTaskDiffCommentRequest,
    ) -> Result<CreateTaskDiffCommentResponse, ApplicationError> {
        self.create_comment.handle(request)
    }

    /// Adds one reply to an existing task diff discussion message.
    pub fn reply_comment(
        &self,
        request: ReplyTaskDiffCommentRequest,
    ) -> Result<ReplyTaskDiffCommentResponse, ApplicationError> {
        self.reply_comment.handle(request)
    }

    /// Resolves or reopens one root task diff discussion.
    pub fn set_comment_status(
        &self,
        request: SetTaskDiffCommentStatusRequest,
    ) -> Result<SetTaskDiffCommentStatusResponse, ApplicationError> {
        self.set_comment_status.handle(request)
    }
}
