use super::{
    DiffRequest, DiffResponse, DiffScope, MAX_DIFF_BYTES, MAX_DIFF_STDERR_BYTES,
    build_tracked_diff_command, build_untracked_command, isolated_git_dir, map_bounded_diff_error,
};
use crate::domain::refs::CommitId;
use crate::error::{DomainError, GitlancerError};
use crate::exec::command::GitCommand;
use crate::exec::runner::GitRunner;
use crate::git::Git;
use crate::git::checkpoint::{ChangeStatus, ChangedPath};
use crate::parse::checkpoint::parse_name_status_z;
use std::collections::BTreeMap;
use std::path::{Component, Path};

const PREVIEW_CONTEXT: &str = "--unified=3";

/// One entry in the review list, with the source path for a rename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffFile {
    pub path: String,
    pub old_path: Option<String>,
    pub status: String,
}

/// Changed paths from the same Git comparison used for a workspace patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffFilesResponse {
    pub head_commit_id: CommitId,
    pub files: Vec<DiffFile>,
}

impl<R: GitRunner> Git<R> {
    /// Lists changed paths without generating or reading the patch bodies.
    pub fn diff_files(
        &self,
        request: DiffRequest<'_>,
    ) -> Result<DiffFilesResponse, GitlancerError> {
        let head_commit_id = self.diff_head(request.worktree)?;
        let tracked =
            self.review_output(&build_tracked_diff_command(&request, &[], "--name-status"))?;
        let mut files = BTreeMap::new();
        for ChangedPath { path, status, .. } in parse_name_status_z(&tracked)? {
            let (status, old_path) = match status {
                ChangeStatus::Added => ("A".to_string(), None),
                ChangeStatus::Modified => ("M".to_string(), None),
                ChangeStatus::Deleted => ("D".to_string(), None),
                ChangeStatus::Renamed { from } => ("R".to_string(), Some(from)),
                ChangeStatus::Other(code) => (code, None),
            };
            files.insert(
                path.clone(),
                DiffFile {
                    path,
                    old_path,
                    status,
                },
            );
        }
        if includes_untracked(request.scope) {
            let paths = self.review_output(&build_untracked_command(request.worktree, &[]))?;
            for path in paths.split_terminator('\0') {
                files.entry(path.to_owned()).or_insert_with(|| DiffFile {
                    path: path.to_owned(),
                    old_path: None,
                    status: "A".to_owned(),
                });
            }
        }
        Ok(DiffFilesResponse {
            head_commit_id,
            files: files.into_values().collect(),
        })
    }

    /// Reads a compact patch for the selected path, including the source side of a rename.
    pub fn diff_file(
        &self,
        request: DiffRequest<'_>,
        path: &str,
        old_path: Option<&str>,
    ) -> Result<DiffResponse, GitlancerError> {
        let mut command = build_tracked_diff_command(&request, &[], PREVIEW_CONTEXT);
        let pathspec = review_pathspec(&request, path)?;
        command.args.push(pathspec.clone());
        if let Some(old_path) = old_path {
            command.args.push(review_pathspec(&request, old_path)?);
        }
        let head_commit_id = self.diff_head(request.worktree)?;
        let mut patch = self.review_output(&command)?;
        if includes_untracked(request.scope) {
            let mut command = build_untracked_command(request.worktree, &[]);
            command.args.extend(["--".to_string(), pathspec]);
            let paths = self.review_output(&command)?;
            // Git's literal pathspec also matches a directory prefix. Only a
            // path returned verbatim by ls-files may reach the no-index reader.
            if paths
                .split_terminator('\0')
                .any(|candidate| candidate == path)
            {
                self.append_untracked(
                    request.worktree,
                    path,
                    &isolated_git_dir(),
                    PREVIEW_CONTEXT,
                    &mut patch,
                )?;
            }
        }
        Ok(DiffResponse {
            head_commit_id,
            patch,
        })
    }

    /// Applies the patch budget and size-error mapping to every review read.
    fn review_output(&self, command: &GitCommand) -> Result<String, GitlancerError> {
        self.runner()
            .run_bounded(command, MAX_DIFF_BYTES, MAX_DIFF_STDERR_BYTES)
            .map(|output| output.stdout)
            .map_err(map_bounded_diff_error)
    }
}

/// Keeps selection inside the checkout and treats Git metacharacters as filename bytes.
fn review_pathspec(request: &DiffRequest<'_>, path: &str) -> Result<String, GitlancerError> {
    let relative = Path::new(path);
    if path.is_empty()
        || path.contains('\0')
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(DomainError::PathOutsideWorktree {
            path: relative.to_path_buf(),
            worktree: request.worktree.worktree_root().as_path().to_path_buf(),
        }
        .into());
    }
    Ok(format!(":(top,literal){path}"))
}

/// Only comparisons against the live worktree include untracked files.
fn includes_untracked(scope: DiffScope) -> bool {
    matches!(scope, DiffScope::Branch | DiffScope::Unstaged)
}
