use super::ports::{
    CommitWorkspaceGitRequest, PushWorkspaceGitRequest, StageWorkspaceGitRequest,
    UnstageWorkspaceGitRequest, WorkspaceGitCommit, WorkspaceGitPush, WorkspaceGitStage,
    WorkspaceGitUnstage, WorkspaceGitWriter, WorkspaceGitWriterError,
};
use gitlancer::domain::worktree::WorktreeHandle;
use gitlancer::git::commit::{AddRequest, CommitRequest, StageAllRequest, UnstageRequest};
use gitlancer::git::worktree::FindWorktreeRequest;
use gitlancer::{CliGitRunner, DomainError, Git, GitlancerError, RepoRoot, Repository};
use ora_utils::path::canonicalize_longest_existing_prefix;
use std::path::Path;
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
        worktree_path: &Path,
        expected_branch_name: &str,
    ) -> Result<WorktreeHandle, WorkspaceGitWriterError> {
        let worktree = self.resolve_worktree(worktree_path)?;
        // Compare canonicalized roots, matching how `find_worktree` itself resolves
        // `worktree_path` against `git worktree list` output: a raw lexical comparison would
        // false-negative on symlink differences (e.g. a symlinked temporary or home directory).
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
        worktree_path: &Path,
    ) -> Result<WorktreeHandle, WorkspaceGitWriterError> {
        self.git
            .find_worktree(FindWorktreeRequest {
                repository: &self.repository,
                candidate_path: worktree_path,
            })
            .map_err(workspace_git_operation_error)
    }

    /// Commits whatever is currently staged in one worktree.
    fn commit_in_worktree(
        &self,
        worktree: &WorktreeHandle,
        message: &str,
    ) -> Result<WorkspaceGitCommit, WorkspaceGitWriterError> {
        self.git
            .commit(CommitRequest {
                worktree,
                message,
                allow_empty: false,
            })
            .map(|response| WorkspaceGitCommit {
                commit_id: response.commit_id.as_str().to_string(),
                summary: response.summary,
            })
            .map_err(workspace_git_operation_error)
    }

    /// Stages the requested repo-relative paths, or every current change when the list is empty.
    fn stage_in_worktree(
        &self,
        worktree: &WorktreeHandle,
        paths: &[String],
    ) -> Result<WorkspaceGitStage, WorkspaceGitWriterError> {
        if paths.is_empty() {
            self.git
                .stage_all(StageAllRequest { worktree })
                .map_err(workspace_git_operation_error)?;
            return Ok(WorkspaceGitStage {
                staged_paths: Vec::new(),
            });
        }

        let repo_paths = paths
            .iter()
            .map(|path| {
                worktree
                    .resolve_repo_relative_path(path)
                    .map_err(workspace_domain_operation_error)
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.git
            .add(AddRequest {
                worktree,
                paths: repo_paths,
            })
            .map_err(workspace_git_operation_error)?;
        Ok(WorkspaceGitStage {
            staged_paths: paths.to_vec(),
        })
    }

    /// Unstages the requested repo-relative paths from one worktree's index.
    fn unstage_in_worktree(
        &self,
        worktree: &WorktreeHandle,
        paths: &[String],
    ) -> Result<WorkspaceGitUnstage, WorkspaceGitWriterError> {
        let repo_paths = paths
            .iter()
            .map(|path| {
                worktree
                    .resolve_repo_relative_path(path)
                    .map_err(workspace_domain_operation_error)
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.git
            .unstage(UnstageRequest {
                worktree,
                paths: repo_paths,
            })
            .map_err(workspace_git_operation_error)?;
        Ok(WorkspaceGitUnstage {
            unstaged_paths: paths.to_vec(),
        })
    }
}

impl WorkspaceGitWriter for GitWorkspaceGitWriter {
    /// Commits the currently staged change set after verifying its recorded branch.
    fn commit_changes(
        &self,
        request: CommitWorkspaceGitRequest,
    ) -> Result<WorkspaceGitCommit, WorkspaceGitWriterError> {
        let worktree =
            self.resolve_verified_worktree(&request.worktree_path, &request.expected_branch_name)?;
        self.commit_in_worktree(&worktree, &request.message)
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

    /// Commits the currently staged changes in a workspace with no recorded branch to verify.
    ///
    /// Used for a project's main checkout, which Ora does not manage the way it manages an
    /// isolated task worktree 鈥?there is no persisted branch name to guard staleness against, so
    /// this trusts whatever is currently checked out.
    fn commit_worktree_changes(
        &self,
        worktree_path: &Path,
        message: &str,
    ) -> Result<WorkspaceGitCommit, WorkspaceGitWriterError> {
        let worktree = self.resolve_worktree(worktree_path)?;
        self.commit_in_worktree(&worktree, message)
    }

    /// Pushes whatever branch is currently checked out in a workspace with no recorded branch.
    ///
    /// See [`Self::commit_worktree_changes`] for why no verification applies here.
    fn push_worktree_branch(
        &self,
        worktree_path: &Path,
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

    /// Stages the supplied paths after verifying its recorded branch.
    fn stage_changes(
        &self,
        request: StageWorkspaceGitRequest,
    ) -> Result<WorkspaceGitStage, WorkspaceGitWriterError> {
        let worktree =
            self.resolve_verified_worktree(&request.worktree_path, &request.expected_branch_name)?;
        self.stage_in_worktree(&worktree, &request.paths)
    }

    /// Unstages the supplied paths after verifying its recorded branch.
    fn unstage_changes(
        &self,
        request: UnstageWorkspaceGitRequest,
    ) -> Result<WorkspaceGitUnstage, WorkspaceGitWriterError> {
        let worktree =
            self.resolve_verified_worktree(&request.worktree_path, &request.expected_branch_name)?;
        self.unstage_in_worktree(&worktree, &request.paths)
    }

    /// Stages the supplied paths in a workspace with no recorded branch to verify.
    fn stage_worktree_changes(
        &self,
        worktree_path: &Path,
        paths: Vec<String>,
    ) -> Result<WorkspaceGitStage, WorkspaceGitWriterError> {
        let worktree = self.resolve_worktree(worktree_path)?;
        self.stage_in_worktree(&worktree, &paths)
    }

    /// Unstages the supplied paths in a workspace with no recorded branch to verify.
    fn unstage_worktree_changes(
        &self,
        worktree_path: &Path,
        paths: Vec<String>,
    ) -> Result<WorkspaceGitUnstage, WorkspaceGitWriterError> {
        let worktree = self.resolve_worktree(worktree_path)?;
        self.unstage_in_worktree(&worktree, &paths)
    }
}

/// Hides Git diagnostics behind the application writer port.
fn workspace_git_operation_error(error: GitlancerError) -> WorkspaceGitWriterError {
    WorkspaceGitWriterError::operation_failed(error)
}

/// Hides path-validation failures behind the application writer port.
fn workspace_domain_operation_error(error: DomainError) -> WorkspaceGitWriterError {
    WorkspaceGitWriterError::operation_failed(GitlancerError::Domain(error))
}
