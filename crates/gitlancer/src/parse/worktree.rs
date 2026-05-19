use std::path::{Path, PathBuf};

use crate::domain::paths::{GitDir, RepoRoot, WorktreeRoot};
use crate::domain::worktree::{WorktreeHandle, WorktreeKind};
use crate::error::ParseError;

/// Parses `git worktree list --porcelain` output into typed worktree handles for one repository.
pub fn parse_worktree_list(
    repo_root: &RepoRoot,
    stdout: &str,
) -> Result<Vec<WorktreeHandle>, ParseError> {
    let mut worktrees = Vec::new();
    let mut current_path: Option<PathBuf> = None;

    for line in stdout.lines() {
        let trimmed = line.trim();

        if let Some(path) = trimmed.strip_prefix("worktree ") {
            if let Some(worktree_root) = current_path.take() {
                worktrees.push(build_worktree_handle(repo_root, worktree_root));
            }

            current_path = Some(PathBuf::from(path));
        }
    }

    if let Some(worktree_root) = current_path.take() {
        worktrees.push(build_worktree_handle(repo_root, worktree_root));
    }

    if worktrees.is_empty() {
        return Err(ParseError::InvalidWorktreeList);
    }

    Ok(worktrees)
}

/// Builds one typed worktree handle from a parsed worktree path and the repository that owns it.
fn build_worktree_handle(repo_root: &RepoRoot, worktree_root: PathBuf) -> WorktreeHandle {
    let kind = if worktree_root == repo_root.as_path() {
        WorktreeKind::Main
    } else {
        WorktreeKind::Linked {
            name: derive_linked_worktree_name(&worktree_root),
        }
    };

    WorktreeHandle::new(
        repo_root.clone(),
        WorktreeRoot::new(&worktree_root),
        GitDir::new(worktree_root.join(".git")),
        kind,
    )
}

/// Derives a stable linked worktree name from its checkout path so runtime APIs can resolve it consistently.
fn derive_linked_worktree_name(worktree_root: &Path) -> String {
    worktree_root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| worktree_root.to_string_lossy().into_owned())
}
