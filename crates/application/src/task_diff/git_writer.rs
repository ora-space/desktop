use super::ports::{
    CommitTaskGitRequest, PushTaskGitRequest, TaskGitCommit, TaskGitPush, TaskGitWriter,
    TaskGitWriterError,
};
use gitlancer::git::commit::{CommitRequest, StageAllRequest};
use gitlancer::git::worktree::FindWorktreeRequest;
use gitlancer::{CliGitRunner, Git, RepoRoot, Repository, WorktreeHandle};
use std::path::PathBuf;

/// Writes commits and pushes through the shared Gitlancer runtime.
#[derive(Clone, Debug)]
pub struct GitTaskGitWriter {
    git: Git<CliGitRunner>,
    repository: Repository,
}

impl GitTaskGitWriter {
    /// Builds a writer for one configured project repository.
    pub fn new(project_root: PathBuf) -> Self {
        Self {
            git: Git::new(CliGitRunner),
            repository: Repository::new(RepoRoot::new(project_root)),
        }
    }

    /// Resolves the exact worktree and verifies its persisted branch before mutation.
    fn resolve_worktree(
        &self,
        worktree_path: &std::path::Path,
        expected_branch_name: &str,
    ) -> Result<WorktreeHandle, TaskGitWriterError> {
        let worktree = self
            .git
            .find_worktree(FindWorktreeRequest {
                repository: &self.repository,
                candidate_path: worktree_path,
            })
            .map_err(task_git_operation_error)?;
        if worktree.worktree_root().as_path() != worktree_path
            || worktree.branch_name().map(gitlancer::BranchName::as_str)
                != Some(expected_branch_name)
        {
            return Err(TaskGitWriterError::OperationFailed(
                "task worktree path or branch no longer matches persisted state".to_string(),
            ));
        }
        Ok(worktree)
    }
}

impl TaskGitWriter for GitTaskGitWriter {
    /// Stages all files before creating a non-empty commit.
    fn commit_changes(
        &self,
        request: CommitTaskGitRequest,
    ) -> Result<TaskGitCommit, TaskGitWriterError> {
        let worktree =
            self.resolve_worktree(&request.worktree_path, &request.expected_branch_name)?;
        self.git
            .stage_all(StageAllRequest {
                worktree: &worktree,
            })
            .map_err(task_git_operation_error)?;
        self.git
            .commit(CommitRequest {
                worktree: &worktree,
                message: &request.message,
                allow_empty: false,
            })
            .map(|response| TaskGitCommit {
                commit_id: response.commit_id.as_str().to_string(),
                summary: response.summary,
            })
            .map_err(task_git_operation_error)
    }

    /// Pushes the exact verified task branch to origin.
    fn push_branch(&self, request: PushTaskGitRequest) -> Result<TaskGitPush, TaskGitWriterError> {
        let worktree =
            self.resolve_worktree(&request.worktree_path, &request.expected_branch_name)?;
        self.git
            .push_branch(&worktree)
            .map(|response| TaskGitPush {
                branch_name: response.branch_name,
                remote_name: response.remote_name,
            })
            .map_err(task_git_operation_error)
    }
}

/// Hides Git diagnostics behind the application writer port.
fn task_git_operation_error(error: gitlancer::GitlancerError) -> TaskGitWriterError {
    TaskGitWriterError::OperationFailed(error.to_string())
}
