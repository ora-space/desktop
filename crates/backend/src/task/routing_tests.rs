use super::{resolve_worktree_error, task_repository_error};
use crate::ErrorClassification;
use gitlancer::{DomainError, GitlancerError};
use pretty_assertions::assert_eq;
use std::io;
use std::path::PathBuf;

/// Git confirming that no worktree owns the branch is real task state, not an outage.
#[test]
fn keeps_an_absent_worktree_a_conflict() {
    let error = resolve_worktree_error(GitlancerError::Domain(DomainError::NotAWorktree(
        PathBuf::from("repo"),
    )));

    assert_eq!(
        (error.classification(), error.public_error().code()),
        (ErrorClassification::Conflict, "task_worktree_unavailable")
    );
}

/// Git itself failing is infrastructure, so it must not be downgraded to a task-state conflict.
#[test]
fn reports_git_execution_faults_as_internal() {
    let error =
        resolve_worktree_error(GitlancerError::Io(io::Error::from(io::ErrorKind::NotFound)));

    assert_eq!(
        (error.classification(), error.public_error().code()),
        (ErrorClassification::Internal, "internal_error")
    );
}

/// A store that cannot answer is an outage; classifying it as task state would hide it at WARN.
#[test]
fn reports_repository_faults_as_internal() {
    let error = task_repository_error(io::Error::from(io::ErrorKind::ResourceBusy));

    assert_eq!(
        (error.classification(), error.public_error().code()),
        (ErrorClassification::Internal, "internal_error")
    );
}
