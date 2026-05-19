use crate::domain::worktree::WorktreeHandle;
use crate::error::GitlancerError;
use crate::exec::command::{GitCommand, GitIntent};
use crate::exec::env::GitEnv;
use crate::exec::runner::GitRunner;
use crate::git::Git;
use crate::parse;

/// Carries the information needed to read structured status information from one worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusRequest<'a> {
    pub worktree: &'a WorktreeHandle,
}

/// Represents the high-level status view returned to upper layers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusResponse {
    pub entries: Vec<StatusEntry>,
}

/// Represents one structured worktree status entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    pub raw: String,
}

impl<R: GitRunner> Git<R> {
    /// Returns worktree status using porcelain v2 so callers can reason about changes without ad-hoc parsing.
    pub fn status(&self, request: StatusRequest<'_>) -> Result<StatusResponse, GitlancerError> {
        let command = GitCommand::new(
            request.worktree.worktree_root().as_path().to_path_buf(),
            vec![
                "status".to_string(),
                "--porcelain=v2".to_string(),
                "-z".to_string(),
            ],
            GitEnv::default(),
            GitIntent::ReadOnly,
        );
        let output = self.runner().run(&command)?;
        let entries = parse::status::parse_status_v2(&output.stdout)?;

        Ok(StatusResponse { entries })
    }
}
