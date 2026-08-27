use super::ports::{CommitWorkspaceGitRequest, PushWorkspaceGitRequest, WorkspaceGitWriter};
use crate::{ApplicationError, WorktreeRepository};
use ora_contracts::{
    CommitWorkspaceChangesRequest, CommitWorkspaceChangesResponse, PushWorkspaceBranchRequest,
    PushWorkspaceBranchResponse,
};
use ora_domain::{WorkspaceId, Worktree};
use std::path::PathBuf;

/// Commits the complete change set from one workspace checkout.
pub struct CommitWorkspaceChangesHandler<WorktreeRepositoryPort, GitWriter> {
    worktree_repository: WorktreeRepositoryPort,
    git_writer: GitWriter,
    worktree_path: PathBuf,
}

impl<WorktreeRepositoryPort, GitWriter>
    CommitWorkspaceChangesHandler<WorktreeRepositoryPort, GitWriter>
{
    /// Builds a commit handler from persistence, Git, and backend path dependencies.
    pub fn new(
        worktree_repository: WorktreeRepositoryPort,
        git_writer: GitWriter,
        worktree_path: PathBuf,
    ) -> Self {
        Self {
            worktree_repository,
            git_writer,
            worktree_path,
        }
    }
}

impl<WorktreeRepositoryPort, GitWriter>
    CommitWorkspaceChangesHandler<WorktreeRepositoryPort, GitWriter>
where
    WorktreeRepositoryPort: WorktreeRepository,
    GitWriter: WorkspaceGitWriter,
{
    /// Validates the message, then commits through the verified path when a `Worktree` row is
    /// recorded for this workspace, or the unguarded path when it is a plain checkout (e.g. a
    /// project's main workspace).
    pub fn handle(
        &self,
        request: CommitWorkspaceChangesRequest,
    ) -> Result<CommitWorkspaceChangesResponse, ApplicationError> {
        let message = request.message.trim();
        if message.is_empty() {
            return Err(ApplicationError::WorkspaceDiffCommitMessageBlank);
        }
        let workspace_id = WorkspaceId::new(request.workspace_id);
        let worktree = load_worktree(&self.worktree_repository, &workspace_id)?;
        let commit = match worktree {
            Some(worktree) => self
                .git_writer
                .commit_changes(CommitWorkspaceGitRequest {
                    worktree_path: self.worktree_path.clone(),
                    expected_branch_name: recorded_branch(&worktree)?.to_string(),
                    message: message.to_string(),
                })
                .map_err(workspace_git_writer_error)?,
            None => self
                .git_writer
                .commit_worktree_changes(&self.worktree_path, message)
                .map_err(workspace_git_writer_error)?,
        };

        Ok(CommitWorkspaceChangesResponse {
            commit_id: commit.commit_id,
            summary: commit.summary,
        })
    }
}

/// Pushes one workspace's checkout branch to its default remote.
pub struct PushWorkspaceBranchHandler<WorktreeRepositoryPort, GitWriter> {
    worktree_repository: WorktreeRepositoryPort,
    git_writer: GitWriter,
    worktree_path: PathBuf,
}

impl<WorktreeRepositoryPort, GitWriter>
    PushWorkspaceBranchHandler<WorktreeRepositoryPort, GitWriter>
{
    /// Builds a push handler from persistence, Git, and backend path dependencies.
    pub fn new(
        worktree_repository: WorktreeRepositoryPort,
        git_writer: GitWriter,
        worktree_path: PathBuf,
    ) -> Self {
        Self {
            worktree_repository,
            git_writer,
            worktree_path,
        }
    }
}

impl<WorktreeRepositoryPort, GitWriter>
    PushWorkspaceBranchHandler<WorktreeRepositoryPort, GitWriter>
where
    WorktreeRepositoryPort: WorktreeRepository,
    GitWriter: WorkspaceGitWriter,
{
    /// Pushes through the verified path when a `Worktree` row is recorded for this workspace, or
    /// the unguarded path when it is a plain checkout (e.g. a project's main workspace).
    pub fn handle(
        &self,
        request: PushWorkspaceBranchRequest,
    ) -> Result<PushWorkspaceBranchResponse, ApplicationError> {
        let workspace_id = WorkspaceId::new(request.workspace_id);
        let worktree = load_worktree(&self.worktree_repository, &workspace_id)?;
        let push = match worktree {
            Some(worktree) => self
                .git_writer
                .push_branch(PushWorkspaceGitRequest {
                    worktree_path: self.worktree_path.clone(),
                    expected_branch_name: recorded_branch(&worktree)?.to_string(),
                })
                .map_err(workspace_git_writer_error)?,
            None => self
                .git_writer
                .push_worktree_branch(&self.worktree_path)
                .map_err(workspace_git_writer_error)?,
        };

        Ok(PushWorkspaceBranchResponse {
            branch_name: push.branch_name,
            remote_name: push.remote_name,
        })
    }
}

/// Returns the persisted branch identity required by a verified write, when one is recorded.
fn recorded_branch(worktree: &Worktree) -> Result<&str, ApplicationError> {
    worktree.branch_name.as_deref().ok_or_else(|| {
        ApplicationError::workspace_diff_failure(std::io::Error::other(
            "recorded worktree has no recorded branch",
        ))
    })
}

/// Converts writer failures into the stable workspace Git error surface.
fn workspace_git_writer_error(error: super::WorkspaceGitWriterError) -> ApplicationError {
    match error {
        super::WorkspaceGitWriterError::OperationFailed(source) => {
            ApplicationError::WorkspaceDiff { source }
        }
    }
}

/// Loads the optional `Worktree` row recorded for a workspace, so commit and push share
/// identical lookup behavior.
fn load_worktree<WorktreeRepositoryPort>(
    worktree_repository: &WorktreeRepositoryPort,
    workspace_id: &WorkspaceId,
) -> Result<Option<Worktree>, ApplicationError>
where
    WorktreeRepositoryPort: WorktreeRepository,
{
    worktree_repository
        .find_worktree(workspace_id)
        .map_err(ApplicationError::from_worktree_repository_error)
}
