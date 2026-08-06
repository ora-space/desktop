use crate::domain::worktree::WorktreeHandle;
use crate::error::{DomainError, GitExecError, GitlancerError};
use crate::exec::command::{GitCommand, GitIntent};
use crate::exec::env::GitEnv;
use crate::exec::runner::GitRunner;
use crate::git::Git;

const DEFAULT_REMOTE_NAME: &str = "origin";

/// Returns the branch and remote updated by a successful push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushBranchResponse {
    pub branch_name: String,
    pub remote_name: String,
}

impl<R: GitRunner> Git<R> {
    /// Pushes the exact checked-out branch to origin without allowing credential prompts.
    pub fn push_branch(
        &self,
        worktree: &WorktreeHandle,
    ) -> Result<PushBranchResponse, GitlancerError> {
        let branch_name = worktree
            .branch_name()
            .ok_or_else(|| {
                GitlancerError::Domain(DomainError::NotAWorktree(
                    worktree.worktree_root().as_path().to_path_buf(),
                ))
            })?
            .as_str();
        let command = GitCommand::new(
            worktree.worktree_root().as_path().to_path_buf(),
            vec![
                "push".to_string(),
                "--set-upstream".to_string(),
                DEFAULT_REMOTE_NAME.to_string(),
                branch_name.to_string(),
            ],
            GitEnv::default().with_variable("GIT_TERMINAL_PROMPT", "0"),
            GitIntent::Network,
        );
        match self.runner().run(&command) {
            Ok(_) => {}
            Err(GitExecError::NonZeroExit {
                code,
                args,
                stdout,
                stderr,
            }) => {
                return Err(GitlancerError::Exec(GitExecError::NonZeroExit {
                    code,
                    args,
                    stdout,
                    stderr: format!(
                        "{stderr}\npush authentication may be unavailable; configure a Git credential helper or HTTPS token"
                    ),
                }));
            }
            Err(error) => return Err(GitlancerError::Exec(error)),
        }

        Ok(PushBranchResponse {
            branch_name: branch_name.to_string(),
            remote_name: DEFAULT_REMOTE_NAME.to_string(),
        })
    }
}
