use std::path::PathBuf;

use thiserror::Error;

/// Represents all public errors returned by the v2 architecture surface.
#[derive(Debug, Error)]
pub enum GitlancerError {
    /// Wraps repository- and worktree-level invariant violations.
    #[error("domain validation failed: {0}")]
    Domain(#[from] DomainError),

    /// Wraps failures produced while invoking the Git CLI.
    #[error("git execution failed: {0}")]
    Exec(#[from] GitExecError),

    /// Wraps failures produced while decoding machine-readable Git output.
    #[error("git output parsing failed: {0}")]
    Parse(#[from] ParseError),
}

/// Represents invalid repository and worktree states detected before execution.
#[derive(Debug, Error)]
pub enum DomainError {
    /// Returned when a path is expected to be a repository root but is not.
    #[error("path is not a repository root: {0:?}")]
    NotARepository(PathBuf),

    /// Returned when a caller names a local branch that does not exist in the supplied repository.
    #[error("branch does not exist in repository {repo:?}: {branch}")]
    BranchNotFound { repo: PathBuf, branch: String },

    /// Returned when a caller attempts to create a local branch that already exists.
    #[error("branch already exists in repository {repo:?}: {branch}")]
    BranchAlreadyExists { repo: PathBuf, branch: String },

    /// Returned when a path is expected to be a worktree root but is not.
    #[error("path is not a worktree root: {0:?}")]
    NotAWorktree(PathBuf),

    /// Returned when a caller tries to delete the repository's main worktree through the linked-worktree lifecycle API.
    #[error("cannot delete the main worktree for repository {0:?}")]
    MainWorktreeDeletionUnsupported(PathBuf),

    /// Returned when a path cannot be safely expressed relative to the worktree root.
    #[error("path {path:?} is outside worktree {worktree:?}")]
    PathOutsideWorktree { path: PathBuf, worktree: PathBuf },

    /// Returned when a worktree does not belong to the repository a caller supplied.
    #[error("worktree {worktree:?} does not belong to repository {repo:?}")]
    WorktreeMismatch { worktree: PathBuf, repo: PathBuf },
}

/// Represents process-level failures produced while invoking the Git CLI.
#[derive(Debug, Error)]
pub enum GitExecError {
    /// Returned when the Git executable is not available on the current PATH.
    #[error("Git executable not found")]
    GitNotFound,

    /// Returned when the process cannot even be spawned.
    #[error("failed to spawn git with args {args:?}: {source}")]
    SpawnFailed {
        args: Vec<String>,
        #[source]
        source: std::io::Error,
    },

    /// Returned when Git exits with a non-zero status code.
    #[error("git exited with code {code:?} for args {args:?}: {stderr}")]
    NonZeroExit {
        code: Option<i32>,
        args: Vec<String>,
        stdout: String,
        stderr: String,
    },
}

/// Represents deterministic failures while decoding Git porcelain or plumbing output.
#[derive(Debug, Error)]
pub enum ParseError {
    /// Returned when a command output is unexpectedly empty.
    #[error("expected at least one non-empty output line")]
    MissingLine,

    /// Returned when the worktree listing cannot be decoded into structured records.
    #[error("invalid worktree list output")]
    InvalidWorktreeList,

    /// Returned when the status listing cannot be decoded into structured records.
    #[error("invalid status output")]
    InvalidStatus,

    /// Returned when a parser slot exists but the typed parser is not implemented yet.
    #[error("parser for feature {feature} is not implemented yet")]
    Unimplemented { feature: &'static str },
}
