use std::path::{Component, Path, PathBuf};

use crate::domain::paths::{GitDir, RepoRelativePath, RepoRoot, WorktreeRoot};
use crate::error::DomainError;

/// Distinguishes the main checkout from linked worktrees because they have different lifecycle semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeKind {
    Main,
    Linked { name: String },
}

/// Represents one executable worktree context that belongs to a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeHandle {
    repo_root: RepoRoot,
    worktree_root: WorktreeRoot,
    git_dir: GitDir,
    kind: WorktreeKind,
}

impl WorktreeHandle {
    /// Creates a worktree handle from validated repository and worktree metadata.
    pub fn new(
        repo_root: RepoRoot,
        worktree_root: WorktreeRoot,
        git_dir: GitDir,
        kind: WorktreeKind,
    ) -> Self {
        Self {
            repo_root,
            worktree_root,
            git_dir,
            kind,
        }
    }

    /// Returns the repository root that owns this worktree.
    pub fn repo_root(&self) -> &RepoRoot {
        &self.repo_root
    }

    /// Returns the checkout root where worktree-scoped Git commands should execute.
    pub fn worktree_root(&self) -> &WorktreeRoot {
        &self.worktree_root
    }

    /// Returns the gitdir backing this worktree so linked worktrees can be handled explicitly.
    pub fn git_dir(&self) -> &GitDir {
        &self.git_dir
    }

    /// Returns the worktree kind so callers can branch on main versus linked behavior deliberately.
    pub fn kind(&self) -> &WorktreeKind {
        &self.kind
    }

    /// Resolves a caller path into a repo-relative path while preventing traversal outside this worktree.
    pub fn resolve_repo_relative_path(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<RepoRelativePath, DomainError> {
        let candidate = path.as_ref();
        let worktree_root = normalize_absolute_path(self.worktree_root.as_path());

        if candidate.is_absolute() {
            let normalized = normalize_absolute_path(candidate);
            let relative = normalized.strip_prefix(&worktree_root).map_err(|_| {
                DomainError::PathOutsideWorktree {
                    path: normalized.clone(),
                    worktree: worktree_root.clone(),
                }
            })?;

            return Ok(RepoRelativePath::new(relative));
        }

        let normalized =
            normalize_relative_path(candidate).ok_or_else(|| DomainError::PathOutsideWorktree {
                path: candidate.to_path_buf(),
                worktree: worktree_root.clone(),
            })?;

        Ok(RepoRelativePath::new(normalized))
    }
}

/// Normalizes an absolute path lexically so containment checks do not depend on filesystem existence.
fn normalize_absolute_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            // Absolute paths cannot traverse above root, so extra `..` segments are ignored there.
            Component::ParentDir => {
                let _ = normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }

    normalized
}

/// Normalizes a relative path and rejects paths whose `..` segments would escape the worktree root.
fn normalize_relative_path(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    let mut depth = 0usize;

    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => return None,
            Component::CurDir => {}
            Component::ParentDir => {
                if depth == 0 {
                    return None;
                }

                let popped = normalized.pop();
                if popped {
                    depth -= 1;
                }
            }
            Component::Normal(part) => {
                normalized.push(part);
                depth += 1;
            }
        }
    }

    Some(normalized)
}
