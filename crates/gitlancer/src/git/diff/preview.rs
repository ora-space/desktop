use super::{
    COMPACT_CONTEXT_ARGS, DiffRequest, DiffResponse, DiffScope, MAX_DIFF_BYTES,
    MAX_DIFF_STDERR_BYTES, REVIEW_DIFF_BYTES, append_patch, build_tracked_diff_command,
    build_untracked_command, isolated_git_dir, map_bounded_diff_error,
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
        // Resolve the comparison and its response label from the same revision.
        let tracked = self.review_output(
            &build_tracked_diff_command(
                &request,
                &[],
                &["--name-status", "-z"],
                head_commit_id.as_str(),
            ),
            MAX_DIFF_BYTES,
        )?;
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
            let paths = self.review_output(
                &build_untracked_command(request.worktree, &[]),
                MAX_DIFF_BYTES,
            )?;
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
        let pathspec = review_pathspec(&request, path)?;
        let old_pathspec = old_path
            .map(|path| review_pathspec(&request, path))
            .transpose()?;
        let head_commit_id = self.diff_head(request.worktree)?;
        let mut patch = if let (Some(old_path), Some(old_pathspec)) =
            (old_path, old_pathspec.as_ref())
            && (is_below(path, old_path) || is_below(old_path, path))
        {
            // A file may be renamed into a directory with its old name. Read
            // the two sides separately so either exclusion can stay exact.
            let old = self.exact_patch(
                &request,
                old_path,
                old_pathspec,
                &head_commit_id,
                REVIEW_DIFF_BYTES,
            )?;
            let separator = usize::from(!old.is_empty() && !old.ends_with('\n'));
            let remaining = REVIEW_DIFF_BYTES.saturating_sub(old.len() + separator);
            let new = self.exact_patch(&request, path, &pathspec, &head_commit_id, remaining)?;
            let mut patch = old;
            append_patch(&mut patch, &new);
            patch
        } else {
            let mut command = build_tracked_diff_command(
                &request,
                &[],
                COMPACT_CONTEXT_ARGS,
                head_commit_id.as_str(),
            );
            select_exact_path(&mut command, path, &pathspec);
            if let (Some(old_path), Some(old_pathspec)) = (old_path, old_pathspec.as_ref()) {
                select_exact_path(&mut command, old_path, old_pathspec);
            }
            self.review_output(&command, REVIEW_DIFF_BYTES)?
        };
        if includes_untracked(request.scope) {
            let mut command = build_untracked_command(request.worktree, &[]);
            command.args.extend(["--".to_string(), pathspec]);
            let paths = self.review_output(&command, MAX_DIFF_BYTES)?;
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
                    COMPACT_CONTEXT_ARGS,
                    REVIEW_DIFF_BYTES,
                    &mut patch,
                )?;
            }
        }
        Ok(DiffResponse {
            head_commit_id,
            patch,
        })
    }

    /// Reads one tracked path without including children of a replaced file.
    fn exact_patch(
        &self,
        request: &DiffRequest<'_>,
        path: &str,
        pathspec: &str,
        head_commit_id: &CommitId,
        max_bytes: usize,
    ) -> Result<String, GitlancerError> {
        let mut command =
            build_tracked_diff_command(request, &[], COMPACT_CONTEXT_ARGS, head_commit_id.as_str());
        select_exact_path(&mut command, path, pathspec);
        self.review_output(&command, max_bytes)
            .map_err(|error| match error {
                GitlancerError::DiffTooLarge { .. } => GitlancerError::DiffTooLarge {
                    byte_count: REVIEW_DIFF_BYTES + 1,
                    max_byte_count: REVIEW_DIFF_BYTES,
                },
                error => error,
            })
    }

    /// Applies the patch budget and size-error mapping to every review read.
    fn review_output(
        &self,
        command: &GitCommand,
        max_bytes: usize,
    ) -> Result<String, GitlancerError> {
        self.runner()
            .run_bounded(command, max_bytes, MAX_DIFF_STDERR_BYTES)
            .map(|output| output.stdout)
            .map_err(|error| map_bounded_diff_error(error, max_bytes))
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

/// Git's literal inclusion also matches descendant paths, so exclude those explicitly.
fn select_exact_path(command: &mut GitCommand, path: &str, pathspec: &str) {
    command.args.push(pathspec.to_owned());
    // A trailing slash also matches a gitlink. A glob with /** targets only descendants.
    let escaped = path
        .replace('\\', "\\\\")
        .replace('*', "\\*")
        .replace('?', "\\?")
        .replace('[', "\\[");
    command
        .args
        .push(format!(":(top,exclude,glob){escaped}/**"));
}

/// Checks whether one repository path is below another path in the Git tree.
fn is_below(path: &str, ancestor: &str) -> bool {
    path.strip_prefix(ancestor)
        .is_some_and(|tail| tail.starts_with('/'))
}

/// Only comparisons against the live worktree include untracked files.
fn includes_untracked(scope: DiffScope) -> bool {
    matches!(scope, DiffScope::Branch | DiffScope::Unstaged)
}
