use crate::domain::worktree::WorktreeHandle;
use crate::exec::command::{GitCommand, GitIntent};
use crate::exec::env::GitEnv;
use crate::exec::runner::GitRunner;
use crate::git::Git;

/// Carries the checked-out worktree and already-resolved upstream ref for a fast-forward pull.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FastForwardRequest<'a> {
    pub worktree: &'a WorktreeHandle,
    pub upstream: &'a str,
}

impl<R: GitRunner> Git<R> {
    /// Advances a worktree to its upstream only when Git can complete a fast-forward update.
    pub fn fast_forward(
        &self,
        request: FastForwardRequest<'_>,
    ) -> Result<(), crate::GitlancerError> {
        self.runner().run(&build_fast_forward_command(&request))?;
        Ok(())
    }
}

/// Builds a local-only fast-forward merge so pull never creates a merge commit implicitly.
pub fn build_fast_forward_command(request: &FastForwardRequest<'_>) -> GitCommand {
    GitCommand::new(
        request.worktree.worktree_root().as_path().to_path_buf(),
        vec![
            "merge".to_string(),
            "--ff-only".to_string(),
            request.upstream.to_string(),
        ],
        GitEnv::default(),
        GitIntent::Mutating,
    )
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::{FastForwardRequest, build_fast_forward_command};
    use crate::domain::paths::{GitDir, RepoRoot, WorktreeRoot};
    use crate::domain::refs::BranchName;
    use crate::domain::worktree::{WorktreeHandle, WorktreeKind};

    /// Verifies pull uses a local fast-forward-only merge and never performs a hidden network call.
    #[test]
    fn builds_fast_forward_command() {
        let root = RepoRoot::new("D:/gitlancer-pull-tests");
        let worktree = WorktreeHandle::new(
            root.clone(),
            WorktreeRoot::new(root.as_path()),
            GitDir::new(root.as_path().join(".git")),
            WorktreeKind::Main,
            Some(BranchName::new("main")),
        );
        let command = build_fast_forward_command(&FastForwardRequest {
            worktree: &worktree,
            upstream: "origin/main",
        });

        assert_eq!(command.args, vec!["merge", "--ff-only", "origin/main"]);
        assert_eq!(command.intent, crate::GitIntent::Mutating);
    }
}
