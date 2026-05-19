use crate::domain::paths::RepoRoot;

/// Represents one repository identity shared by its main and linked worktrees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    root: RepoRoot,
}

impl Repository {
    /// Creates a repository handle from a validated repository root.
    pub fn new(root: RepoRoot) -> Self {
        Self { root }
    }

    /// Returns the repository root so callers can derive worktree-scoped commands from it.
    pub fn root(&self) -> &RepoRoot {
        &self.root
    }
}
