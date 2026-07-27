use super::ports::{
    ReadTaskDiffRequest, ReadTaskDiffScope, TaskDiffReader, TaskDiffReaderError, TaskDiffSnapshot,
};
use gitlancer::git::diff::{DiffRequest, DiffResponse, DiffScope};
use gitlancer::git::worktree::FindWorktreeRequest;
use gitlancer::{CliGitRunner, CommitId, Git, RepoRoot, Repository};
use std::path::PathBuf;

/// Reads task-scoped unified diffs through the shared Gitlancer runtime.
#[derive(Clone, Debug)]
pub struct GitTaskDiffReader {
    git: Git<CliGitRunner>,
    repository: Repository,
}

impl GitTaskDiffReader {
    /// Builds a Git-backed reader for one configured project repository.
    pub fn new(project_root: PathBuf) -> Self {
        Self {
            git: Git::new(CliGitRunner),
            repository: Repository::new(RepoRoot::new(project_root)),
        }
    }
}

impl TaskDiffReader for GitTaskDiffReader {
    /// Resolves the backend-owned worktree before computing its fixed-baseline diff.
    fn read_task_diff(
        &self,
        request: ReadTaskDiffRequest,
    ) -> Result<TaskDiffSnapshot, TaskDiffReaderError> {
        let worktree = self
            .git
            .find_worktree(FindWorktreeRequest {
                repository: &self.repository,
                candidate_path: &request.worktree_path,
            })
            .map_err(task_diff_operation_error)?;
        let base_commit_id = CommitId::new(request.base_commit_id);

        self.git
            .diff(DiffRequest {
                worktree: &worktree,
                base_commit_id: &base_commit_id,
                scope: map_diff_scope(request.scope),
            })
            .map(map_diff_response)
            .map_err(task_diff_operation_error)
    }
}

/// Maps the application comparison choice into Gitlancer's command vocabulary.
fn map_diff_scope(scope: ReadTaskDiffScope) -> DiffScope {
    match scope {
        ReadTaskDiffScope::Branch => DiffScope::Branch,
        ReadTaskDiffScope::Unstaged => DiffScope::Unstaged,
        ReadTaskDiffScope::Staged => DiffScope::Staged,
        ReadTaskDiffScope::Committed => DiffScope::Committed,
    }
}

/// Maps Gitlancer's internal response into the application-owned snapshot.
fn map_diff_response(response: DiffResponse) -> TaskDiffSnapshot {
    TaskDiffSnapshot {
        head_commit_id: response.head_commit_id.as_str().to_string(),
        patch: response.patch,
    }
}

/// Hides Git and filesystem diagnostics behind a stable application-port error.
fn task_diff_operation_error(error: gitlancer::GitlancerError) -> TaskDiffReaderError {
    match error {
        gitlancer::GitlancerError::DiffTooLarge {
            byte_count,
            max_byte_count,
        } => TaskDiffReaderError::TooLarge {
            byte_count,
            max_byte_count,
        },
        error => TaskDiffReaderError::OperationFailed(error.to_string()),
    }
}
