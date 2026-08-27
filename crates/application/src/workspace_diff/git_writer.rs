use super::ports::{
    CommitWorkspaceGitRequest, PushWorkspaceGitRequest, WorkspaceGitCommit, WorkspaceGitPush,
    WorkspaceGitWriter, WorkspaceGitWriterError,
};
use gitlancer::git::commit::{CommitRequest, StageAllRequest};
use gitlancer::git::worktree::FindWorktreeRequest;
use gitlancer::{CliGitRunner, Git, RepoRoot, Repository, WorktreeHandle};
use ora_utils::path::canonicalize_longest_existing_prefix;
use std::path::PathBuf;

/// Writes commits and pushes through the shared Gitlancer runtime.
#[derive(Clone, Debug)]
pub struct GitWorkspaceGitWriter {
    git: Git<CliGitRunner>,
    repository: Repository,
}

impl GitWorkspaceGitWriter {
    /// Builds a writer for one configured project repository.
    pub fn new(project_root: PathBuf) -> Self {
        Self {
            git: Git::new(CliGitRunner),
            repository: Repository::new(RepoRoot::new(project_root)),
        }
    }

    /// Resolves the exact worktree and verifies its persisted branch before mutation.
    fn resolve_verified_worktree(
        &self,
        worktree_path: &std::path::Path,
        expected_branch_name: &str,
    ) -> Result<WorktreeHandle, WorkspaceGitWriterError> {
        let worktree = self.resolve_worktree(worktree_path)?;
        // Compare canonicalized roots, matching how `find_worktree` itself resolves
        // `worktree_path` against `git worktree list` output: a raw lexical comparison would
        // false-negative on symlink differences (e.g. macOS `/tmp` -> `/private/tmp`).
        if canonicalize_longest_existing_prefix(worktree.worktree_root().as_path())
            != canonicalize_longest_existing_prefix(worktree_path)
            || worktree.branch_name().map(gitlancer::BranchName::as_str)
                != Some(expected_branch_name)
        {
            return Err(WorkspaceGitWriterError::operation_failed(
                std::io::Error::other(
                    "workspace worktree path or branch no longer matches persisted state",
                ),
            ));
        }
        Ok(worktree)
    }

    /// Resolves the exact worktree without a persisted branch to verify it against.
    fn resolve_worktree(
        &self,
        worktree_path: &std::path::Path,
    ) -> Result<WorktreeHandle, WorkspaceGitWriterError> {
        self.git
            .find_worktree(FindWorktreeRequest {
                repository: &self.repository,
                candidate_path: worktree_path,
            })
            .map_err(workspace_git_operation_error)
    }
}

impl WorkspaceGitWriter for GitWorkspaceGitWriter {
    /// Stages and commits every current worktree change after verifying its recorded branch.
    fn commit_changes(
        &self,
        request: CommitWorkspaceGitRequest,
    ) -> Result<WorkspaceGitCommit, WorkspaceGitWriterError> {
        let worktree =
            self.resolve_verified_worktree(&request.worktree_path, &request.expected_branch_name)?;
        self.git
            .stage_all(StageAllRequest {
                worktree: &worktree,
            })
            .map_err(workspace_git_operation_error)?;
        self.git
            .commit(CommitRequest {
                worktree: &worktree,
                message: &request.message,
                allow_empty: false,
            })
            .map(|response| WorkspaceGitCommit {
                commit_id: response.commit_id.as_str().to_string(),
                summary: response.summary,
            })
            .map_err(workspace_git_operation_error)
    }

    /// Pushes the exact verified workspace branch to origin.
    fn push_branch(
        &self,
        request: PushWorkspaceGitRequest,
    ) -> Result<WorkspaceGitPush, WorkspaceGitWriterError> {
        let worktree =
            self.resolve_verified_worktree(&request.worktree_path, &request.expected_branch_name)?;
        self.git
            .push_branch(&worktree)
            .map(|response| WorkspaceGitPush {
                branch_name: response.branch_name,
                remote_name: response.remote_name,
            })
            .map_err(workspace_git_operation_error)
    }

    /// Stages and commits every current change in a workspace with no recorded branch to verify.
    ///
    /// Used for a project's main checkout, which Ora does not manage the way it manages an
    /// isolated task worktree — there is no persisted branch name to guard staleness against, so
    /// this trusts whatever is currently checked out.
    fn commit_worktree_changes(
        &self,
        worktree_path: &std::path::Path,
        message: &str,
    ) -> Result<WorkspaceGitCommit, WorkspaceGitWriterError> {
        let worktree = self.resolve_worktree(worktree_path)?;
        self.git
            .stage_all(StageAllRequest {
                worktree: &worktree,
            })
            .map_err(workspace_git_operation_error)?;
        self.git
            .commit(CommitRequest {
                worktree: &worktree,
                message,
                allow_empty: false,
            })
            .map(|response| WorkspaceGitCommit {
                commit_id: response.commit_id.as_str().to_string(),
                summary: response.summary,
            })
            .map_err(workspace_git_operation_error)
    }

    /// Pushes whatever branch is currently checked out in a workspace with no recorded branch.
    ///
    /// See [`Self::commit_worktree_changes`] for why no verification applies here.
    fn push_worktree_branch(
        &self,
        worktree_path: &std::path::Path,
    ) -> Result<WorkspaceGitPush, WorkspaceGitWriterError> {
        let worktree = self.resolve_worktree(worktree_path)?;
        self.git
            .push_branch(&worktree)
            .map(|response| WorkspaceGitPush {
                branch_name: response.branch_name,
                remote_name: response.remote_name,
            })
            .map_err(workspace_git_operation_error)
    }
}

/// Hides Git diagnostics behind the application writer port.
fn workspace_git_operation_error(error: gitlancer::GitlancerError) -> WorkspaceGitWriterError {
    WorkspaceGitWriterError::operation_failed(error)
}
