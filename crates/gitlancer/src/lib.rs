pub mod domain;
pub mod error;
pub mod exec;
pub mod git;
pub mod parse;

pub use domain::paths::{GitDir, RepoRelativePath, RepoRoot, WorktreeRoot};
pub use domain::refs::{BranchName, CommitId};
pub use domain::repo::Repository;
pub use domain::worktree::{WorktreeHandle, WorktreeKind};
pub use error::{DomainError, GitExecError, GitlancerError, ParseError};
pub use exec::command::{GitCommand, GitIntent};
pub use exec::env::GitEnv;
pub use exec::output::GitOutput;
pub use exec::runner::{CliGitRunner, GitRunner, RecordingGitRunner};
pub use git::Git;
