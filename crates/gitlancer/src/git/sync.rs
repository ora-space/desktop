use std::path::{Path, PathBuf};

use crate::domain::repo::Repository;
use crate::domain::worktree::WorktreeHandle;
use crate::error::{GitExecError, GitlancerError};
use crate::exec::command::{GitCommand, GitIntent};
use crate::exec::env::GitEnv;
use crate::exec::runner::GitRunner;
use crate::git::Git;

/// Identifies the local integration operation currently owned by Git.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncOperation {
    Merge,
    Rebase,
}

/// Describes whether an integration command completed or left Git in a conflict state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncResult {
    Completed,
    Conflicted,
}

/// Carries the repository, worktree, upstream, and selected integration operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrateRequest<'a> {
    pub repository: &'a Repository,
    pub worktree: &'a WorktreeHandle,
    pub upstream: &'a str,
    pub operation: SyncOperation,
}

/// Carries the repository and worktree for a conflict continuation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContinueSyncRequest<'a> {
    pub repository: &'a Repository,
    pub worktree: &'a WorktreeHandle,
    pub operation: SyncOperation,
}

/// Carries the repository and worktree for aborting an active integration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbortSyncRequest<'a> {
    pub repository: &'a Repository,
    pub worktree: &'a WorktreeHandle,
    pub operation: SyncOperation,
}

/// Identifies the repository whose active merge or rebase operation should be inspected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadSyncOperationRequest<'a> {
    pub repository: &'a Repository,
}

impl<R: GitRunner> Git<R> {
    /// Detects an active rebase or merge using Git's own repository metadata paths.
    pub fn read_sync_operation(
        &self,
        request: ReadSyncOperationRequest<'_>,
    ) -> Result<Option<SyncOperation>, GitlancerError> {
        let rebase_merge = self.read_git_path(request.repository, "rebase-merge")?;
        let rebase_apply = self.read_git_path(request.repository, "rebase-apply")?;
        if rebase_merge.is_dir() || rebase_apply.is_dir() {
            return Ok(Some(SyncOperation::Rebase));
        }

        let merge_head = self.read_git_path(request.repository, "MERGE_HEAD")?;
        if merge_head.is_file() {
            return Ok(Some(SyncOperation::Merge));
        }

        Ok(None)
    }

    /// Starts a merge or rebase and converts an expected conflict into a typed result.
    pub fn integrate(&self, request: IntegrateRequest<'_>) -> Result<SyncResult, GitlancerError> {
        let command = build_integrate_command(&request);
        self.run_with_conflict_detection(&command, request.repository)
    }

    /// Continues an active merge or rebase after the caller has staged resolved paths.
    pub fn continue_sync(
        &self,
        request: ContinueSyncRequest<'_>,
    ) -> Result<SyncResult, GitlancerError> {
        let command = build_continue_command(&request);
        self.run_with_conflict_detection(&command, request.repository)
    }

    /// Aborts an active merge or rebase and restores the pre-integration checkout.
    pub fn abort_sync(&self, request: AbortSyncRequest<'_>) -> Result<(), GitlancerError> {
        self.runner().run(&build_abort_command(&request))?;
        Ok(())
    }

    /// Reads one Git metadata path so worktree-specific repositories resolve their own state.
    fn read_git_path(
        &self,
        repository: &Repository,
        path: &str,
    ) -> Result<PathBuf, GitlancerError> {
        let output = self.runner().run(&GitCommand::new(
            repository.root().as_path().to_path_buf(),
            vec![
                "rev-parse".to_string(),
                "--git-path".to_string(),
                path.to_string(),
            ],
            GitEnv::default(),
            GitIntent::ReadOnly,
        ))?;
        let path = Path::new(output.stdout.trim());
        Ok(if path.is_absolute() {
            path.to_path_buf()
        } else {
            repository.root().as_path().join(path)
        })
    }

    /// Distinguishes expected conflict stops from unrelated Git command failures.
    fn run_with_conflict_detection(
        &self,
        command: &GitCommand,
        repository: &Repository,
    ) -> Result<SyncResult, GitlancerError> {
        match self.runner().run(command) {
            Ok(_) => Ok(SyncResult::Completed),
            Err(error @ GitExecError::NonZeroExit { .. }) => {
                if self
                    .read_sync_operation(ReadSyncOperationRequest { repository })?
                    .is_some()
                {
                    Ok(SyncResult::Conflicted)
                } else {
                    Err(error.into())
                }
            }
            Err(error) => Err(error.into()),
        }
    }
}

/// Builds the selected merge or rebase command with prompts disabled.
pub fn build_integrate_command(request: &IntegrateRequest<'_>) -> GitCommand {
    let args = match request.operation {
        SyncOperation::Merge => vec![
            "merge".to_string(),
            "--no-edit".to_string(),
            request.upstream.to_string(),
        ],
        SyncOperation::Rebase => vec!["rebase".to_string(), request.upstream.to_string()],
    };
    GitCommand::new(
        request.worktree.worktree_root().as_path().to_path_buf(),
        args,
        network_safe_env(),
        GitIntent::Mutating,
    )
}

/// Builds the operation-specific continuation command with the default message editor disabled.
pub fn build_continue_command(request: &ContinueSyncRequest<'_>) -> GitCommand {
    let subcommand = match request.operation {
        SyncOperation::Merge => "merge",
        SyncOperation::Rebase => "rebase",
    };
    GitCommand::new(
        request.worktree.worktree_root().as_path().to_path_buf(),
        vec![subcommand.to_string(), "--continue".to_string()],
        continue_env(),
        GitIntent::Mutating,
    )
}

/// Builds the operation-specific abort command.
pub fn build_abort_command(request: &AbortSyncRequest<'_>) -> GitCommand {
    let subcommand = match request.operation {
        SyncOperation::Merge => "merge",
        SyncOperation::Rebase => "rebase",
    };
    GitCommand::new(
        request.worktree.worktree_root().as_path().to_path_buf(),
        vec![subcommand.to_string(), "--abort".to_string()],
        GitEnv::default(),
        GitIntent::Mutating,
    )
}

/// Keeps network-capable integration commands deterministic by rejecting terminal prompts.
fn network_safe_env() -> GitEnv {
    GitEnv::default().with_variable("GIT_TERMINAL_PROMPT", "0")
}

/// Keeps continuation commands from blocking on an editor while preserving Git's generated message.
fn continue_env() -> GitEnv {
    network_safe_env().with_variable("GIT_EDITOR", "true")
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::{
        AbortSyncRequest, ContinueSyncRequest, IntegrateRequest, SyncOperation,
        build_abort_command, build_continue_command, build_integrate_command,
    };
    use crate::domain::paths::{GitDir, RepoRoot, WorktreeRoot};
    use crate::domain::refs::BranchName;
    use crate::domain::repo::Repository;
    use crate::domain::worktree::{WorktreeHandle, WorktreeKind};

    /// Creates a stable repository/worktree fixture for command assembly tests.
    fn fixture() -> (Repository, WorktreeHandle) {
        let root = RepoRoot::new("D:/gitlancer-sync-tests");
        let repository = Repository::new(root.clone());
        let worktree = WorktreeHandle::new(
            root.clone(),
            WorktreeRoot::new(root.as_path()),
            GitDir::new(root.as_path().join(".git")),
            WorktreeKind::Main,
            Some(BranchName::new("main")),
        );
        (repository, worktree)
    }

    /// Verifies merge and rebase command assembly keeps the selected upstream explicit.
    #[test]
    fn builds_integration_commands() {
        let (repository, worktree) = fixture();
        let merge = build_integrate_command(&IntegrateRequest {
            repository: &repository,
            worktree: &worktree,
            upstream: "origin/main",
            operation: SyncOperation::Merge,
        });
        let rebase = build_integrate_command(&IntegrateRequest {
            repository: &repository,
            worktree: &worktree,
            upstream: "origin/main",
            operation: SyncOperation::Rebase,
        });

        assert_eq!(merge.args, vec!["merge", "--no-edit", "origin/main"]);
        assert_eq!(rebase.args, vec!["rebase", "origin/main"]);
        assert_eq!(merge.intent, crate::GitIntent::Mutating);
        assert_eq!(rebase.intent, crate::GitIntent::Mutating);
    }

    /// Verifies continue and abort use the selected operation without accepting raw subcommands.
    #[test]
    fn builds_resolution_commands() {
        let (repository, worktree) = fixture();
        let continue_command = build_continue_command(&ContinueSyncRequest {
            repository: &repository,
            worktree: &worktree,
            operation: SyncOperation::Rebase,
        });
        let abort_command = build_abort_command(&AbortSyncRequest {
            repository: &repository,
            worktree: &worktree,
            operation: SyncOperation::Merge,
        });

        assert_eq!(continue_command.args, vec!["rebase", "--continue"]);
        assert_eq!(abort_command.args, vec!["merge", "--abort"]);
        assert_eq!(
            continue_command.env.variables.get("GIT_EDITOR"),
            Some(&"true".to_string())
        );
    }
}
