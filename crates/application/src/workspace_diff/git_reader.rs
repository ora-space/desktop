use super::ports::{
    ReadWorkspaceDiffRequest, ReadWorkspaceDiffScope, ReadWorkspaceDiffSelection,
    WorkspaceDiffFile, WorkspaceDiffReader, WorkspaceDiffReaderError, WorkspaceDiffSnapshot,
    WorkspaceStatusFile, WorkspaceStatusSnapshot,
};
use gitlancer::git::diff::{DiffRequest, DiffResponse, DiffScope};
use gitlancer::git::status::StatusEntriesRequest;
use gitlancer::git::worktree::FindWorktreeRequest;
use gitlancer::{CliGitRunner, CommitId, Git, RepoRoot, Repository};
use std::path::Path;
use std::path::PathBuf;

/// Reads workspace-scoped unified diffs through the shared Gitlancer runtime.
#[derive(Clone, Debug)]
pub struct GitWorkspaceDiffReader {
    git: Git<CliGitRunner>,
    repository: Repository,
}

impl GitWorkspaceDiffReader {
    /// Builds a Git-backed reader for one configured project repository.
    pub fn new(project_root: PathBuf) -> Self {
        Self {
            git: Git::new(CliGitRunner),
            repository: Repository::new(RepoRoot::new(project_root)),
        }
    }
}

impl WorkspaceDiffReader for GitWorkspaceDiffReader {
    /// Resolves the backend-owned worktree before computing its diff against the given baseline.
    fn read_workspace_diff(
        &self,
        request: ReadWorkspaceDiffRequest,
    ) -> Result<WorkspaceDiffSnapshot, WorkspaceDiffReaderError> {
        let worktree = self
            .git
            .find_worktree(FindWorktreeRequest {
                repository: &self.repository,
                candidate_path: &request.worktree_path,
            })
            .map_err(workspace_diff_operation_error)?;
        let base_commit_id = request.base_commit_id.map(CommitId::new);
        let scope = map_diff_scope(request.scope);
        // Defense in depth: `Git::diff` panics if `Branch`/`Committed` reach it without a
        // baseline. The backend rejects that combination before calling this reader, but any
        // other caller (a future adapter, a direct consumer of this crate) must get an error
        // here instead of a crash.
        if base_commit_id.is_none() && scope_requires_baseline(scope) {
            return Err(WorkspaceDiffReaderError::operation_failed(
                std::io::Error::other(
                    "Branch/Committed diff scope requires a recorded baseline commit",
                ),
            ));
        }

        let diff = DiffRequest {
            worktree: &worktree,
            base_commit_id: base_commit_id.as_ref(),
            scope,
        };
        match request.selection {
            ReadWorkspaceDiffSelection::Patch => self.git.diff(diff).map(map_diff_response),
            ReadWorkspaceDiffSelection::File { path, old_path } => self
                .git
                .diff_file(diff, &path, old_path.as_deref())
                .map(map_diff_response),
            ReadWorkspaceDiffSelection::Files => {
                self.git
                    .diff_files(diff)
                    .map(|response| WorkspaceDiffSnapshot {
                        head_commit_id: response.head_commit_id.as_str().to_string(),
                        patch: String::new(),
                        files: Some(
                            response
                                .files
                                .into_iter()
                                .map(|file| WorkspaceDiffFile {
                                    path: file.path,
                                    old_path: file.old_path,
                                    status: file.status,
                                })
                                .collect(),
                        ),
                    })
            }
        }
        .map_err(workspace_diff_operation_error)
    }

    /// Resolves the backend-owned worktree before reading its structured per-file status.
    fn read_workspace_status(
        &self,
        worktree_path: &Path,
    ) -> Result<WorkspaceStatusSnapshot, WorkspaceDiffReaderError> {
        let worktree = self
            .git
            .find_worktree(FindWorktreeRequest {
                repository: &self.repository,
                candidate_path: worktree_path,
            })
            .map_err(workspace_diff_operation_error)?;

        self.git
            .status_entries(StatusEntriesRequest {
                worktree: &worktree,
            })
            .map(|response| WorkspaceStatusSnapshot {
                entries: response
                    .entries
                    .into_iter()
                    .map(|entry| WorkspaceStatusFile {
                        path: entry.path,
                        is_staged: entry.is_staged,
                        is_untracked: entry.is_untracked,
                    })
                    .collect(),
            })
            .map_err(workspace_diff_operation_error)
    }
}

/// Reports whether a scope compares against a fixed baseline commit.
fn scope_requires_baseline(scope: DiffScope) -> bool {
    matches!(scope, DiffScope::Branch | DiffScope::Committed)
}

/// Maps the application comparison choice into Gitlancer's command vocabulary.
fn map_diff_scope(scope: ReadWorkspaceDiffScope) -> DiffScope {
    match scope {
        ReadWorkspaceDiffScope::Branch => DiffScope::Branch,
        ReadWorkspaceDiffScope::Unstaged => DiffScope::Unstaged,
        ReadWorkspaceDiffScope::Staged => DiffScope::Staged,
        ReadWorkspaceDiffScope::Committed => DiffScope::Committed,
    }
}

/// Maps Gitlancer's internal response into the application-owned snapshot.
fn map_diff_response(response: DiffResponse) -> WorkspaceDiffSnapshot {
    WorkspaceDiffSnapshot {
        head_commit_id: response.head_commit_id.as_str().to_string(),
        patch: response.patch,
        files: None,
    }
}

/// Hides Git and filesystem diagnostics behind a stable application-port error.
fn workspace_diff_operation_error(error: gitlancer::GitlancerError) -> WorkspaceDiffReaderError {
    match error {
        gitlancer::GitlancerError::DiffTooLarge {
            byte_count,
            max_byte_count,
        } => WorkspaceDiffReaderError::TooLarge {
            byte_count,
            max_byte_count,
        },
        error => WorkspaceDiffReaderError::operation_failed(error),
    }
}
