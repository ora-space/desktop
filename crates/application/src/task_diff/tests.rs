use super::handlers::{stable_diff_id, validate_anchor, validate_comment_body};
use crate::ApplicationError;
use pretty_assertions::assert_eq;

/// Verifies snapshot identifiers stay stable for identical revision content.
#[test]
fn creates_stable_diff_identifiers() {
    assert_eq!(
        stable_diff_id("base", "head", "patch"),
        stable_diff_id("base", "head", "patch")
    );
    assert_ne!(
        stable_diff_id("base", "head", "patch"),
        stable_diff_id("base", "head", "changed patch")
    );
}

/// Verifies comment bodies cannot be empty after whitespace normalization.
#[test]
fn rejects_blank_comment_bodies() {
    assert_eq!(
        validate_comment_body(" \n "),
        Err(ApplicationError::TaskDiffCommentInvalid {
            message: "comment body must not be blank".to_string(),
        })
    );
}

/// Verifies anchors cannot escape the task worktree through parent components.
#[test]
fn rejects_parent_relative_comment_paths() {
    assert_eq!(
        validate_anchor("diff-1", "../secret.txt", 1, 1),
        Err(ApplicationError::TaskDiffCommentInvalid {
            message: "comment anchor is invalid".to_string(),
        })
    );
}
