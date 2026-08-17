use std::path::Path;

use crate::domain::paths::{GitDir, RepoRelativePath, RepoRoot, WorktreeRoot};
use crate::domain::refs::BranchName;
use crate::error::DomainError;
use ora_utils::path::{normalize_absolute, normalize_relative};

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
    branch_name: Option<BranchName>,
}

impl WorktreeHandle {
    /// Creates a worktree handle from validated repository and worktree metadata.
    pub fn new(
        repo_root: RepoRoot,
        worktree_root: WorktreeRoot,
        git_dir: GitDir,
        kind: WorktreeKind,
        branch_name: Option<BranchName>,
    ) -> Self {
        Self {
            repo_root,
            worktree_root,
            git_dir,
            kind,
            branch_name,
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

    /// Returns the checked-out branch reported by Git, or `None` for detached worktrees.
    pub fn branch_name(&self) -> Option<&BranchName> {
        self.branch_name.as_ref()
    }

    /// Resolves a caller path into a repo-relative path while preventing traversal outside this worktree.
    pub fn resolve_repo_relative_path(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<RepoRelativePath, DomainError> {
        let candidate = path.as_ref();
        let worktree_root = normalize_absolute(self.worktree_root.as_path());

        if candidate.is_absolute() {
            let normalized = normalize_absolute(candidate);
            let relative = normalized.strip_prefix(&worktree_root).map_err(|_| {
                DomainError::PathOutsideWorktree {
                    path: normalized.clone(),
                    worktree: worktree_root.clone(),
                }
            })?;

            return Ok(RepoRelativePath::new(relative));
        }

        let normalized =
            normalize_relative(candidate).ok_or_else(|| DomainError::PathOutsideWorktree {
                path: candidate.to_path_buf(),
                worktree: worktree_root.clone(),
            })?;

        Ok(RepoRelativePath::new(normalized))
    }
}
