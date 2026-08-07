use crate::domain::paths::RepoRelativePath;
use crate::domain::worktree::WorktreeHandle;
use crate::error::GitlancerError;
use crate::exec::command::{GitCommand, GitIntent};
use crate::exec::env::GitEnv;
use crate::exec::runner::GitRunner;
use crate::git::Git;

/// Selects one side of an unmerged path before it is staged as the resolved result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictSide {
    Ours,
    Theirs,
}

/// Carries the worktree, validated path, and side selected for one conflict resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveConflictRequest<'a> {
    pub worktree: &'a WorktreeHandle,
    pub path: &'a RepoRelativePath,
    pub side: ConflictSide,
}

impl<R: GitRunner> Git<R> {
    /// Replaces one conflicted path with a selected Git side and stages the resolved result.
    pub fn resolve_conflict(
        &self,
        request: ResolveConflictRequest<'_>,
    ) -> Result<(), GitlancerError> {
        self.runner()
            .run(&build_checkout_conflict_command(&request))?;
        self.runner().run(&build_stage_conflict_command(&request))?;
        Ok(())
    }
}

/// Builds the side-selection command without accepting an arbitrary Git option from the caller.
pub fn build_checkout_conflict_command(request: &ResolveConflictRequest<'_>) -> GitCommand {
    GitCommand::new(
        request.worktree.worktree_root().as_path().to_path_buf(),
        vec![
            "checkout".to_string(),
            request.side.as_arg().to_string(),
            "--".to_string(),
            request.path.as_path().to_string_lossy().into_owned(),
        ],
        GitEnv::default(),
        GitIntent::Mutating,
    )
}

/// Builds the explicit staging command that turns the selected side into a resolved index entry.
pub fn build_stage_conflict_command(request: &ResolveConflictRequest<'_>) -> GitCommand {
    GitCommand::new(
        request.worktree.worktree_root().as_path().to_path_buf(),
        vec![
            "add".to_string(),
            "--".to_string(),
            request.path.as_path().to_string_lossy().into_owned(),
        ],
        GitEnv::default(),
        GitIntent::Mutating,
    )
}

impl ConflictSide {
    /// Returns the Git checkout flag corresponding to the typed conflict side.
    fn as_arg(self) -> &'static str {
        match self {
            Self::Ours => "--ours",
            Self::Theirs => "--theirs",
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::{
        ConflictSide, ResolveConflictRequest, build_checkout_conflict_command,
        build_stage_conflict_command,
    };
    use crate::domain::paths::{GitDir, RepoRelativePath, RepoRoot, WorktreeRoot};
    use crate::domain::refs::BranchName;
    use crate::domain::worktree::{WorktreeHandle, WorktreeKind};

    /// Creates a stable main-worktree fixture for conflict command assembly tests.
    fn worktree_fixture() -> WorktreeHandle {
        let root = RepoRoot::new("D:/gitlancer-conflict-tests");
        WorktreeHandle::new(
            root.clone(),
            WorktreeRoot::new(root.as_path()),
            GitDir::new(root.as_path().join(".git")),
            WorktreeKind::Main,
            Some(BranchName::new("main")),
        )
    }

    /// Verifies side selection and staging remain separate, explicit mutating commands.
    #[test]
    fn builds_side_selection_and_stage_commands() {
        let worktree = worktree_fixture();
        let path = RepoRelativePath::new("src/main.rs");
        let request = ResolveConflictRequest {
            worktree: &worktree,
            path: &path,
            side: ConflictSide::Theirs,
        };

        assert_eq!(
            build_checkout_conflict_command(&request).args,
            vec!["checkout", "--theirs", "--", "src/main.rs"]
        );
        assert_eq!(
            build_stage_conflict_command(&request).args,
            vec!["add", "--", "src/main.rs"]
        );
    }
}
