use super::validate_commit;
use crate::{
    CloneExecutionSpec, CommitId, MessageValidationError, NodePath, NodeRuntimeIdentity,
    RepositoryId,
};
use serde::{Deserialize, Serialize};

/// Successful acquisition facts; the original spec carries the snapshot input, and `commit`
/// always equals its `base_commit`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CloneReady {
    pub node: NodeRuntimeIdentity,
    pub spec: CloneExecutionSpec,
    pub repository_id: RepositoryId,
    pub path: NodePath,
    pub commit: CommitId,
}

/// Definitive failure categories contain no raw Git diagnostics or credentials.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CloneFailureCode {
    /// The Git Server was unreachable, or answered 5xx, until the retries ran out.
    SourceUnavailable,
    /// Only in records written before the snapshot input; a new result carrying it is rejected.
    BranchNotFound,
    DestinationConflict,
    OperationFailed,
    /// Terminated before Git reached its own verdict, with cleanup confirmed; a new execution may
    /// simply retry, unlike `OperationFailed`, which usually needs investigation first.
    Interrupted,
    /// The Git Server answered that the snapshot ref is missing or not readable; it gives both
    /// the same answer.
    SnapshotUnavailable,
    /// The fetched snapshot ref was not a commit, or not the input's base commit: the Git Server
    /// and Cloud disagree about the snapshot, which a network failure must not hide.
    BaseCommitMismatch,
}

/// Describes retained responsibility, never permission to delete or reuse a directory.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CloneResidual {
    NoDirectory {},
    Retained {
        repository_id: RepositoryId,
        path: NodePath,
    },
}

/// A known failed attempt with confirmed cleanup; uncertain outcomes remain ExecutionState::Unknown.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CloneFailed {
    pub node: NodeRuntimeIdentity,
    pub spec: CloneExecutionSpec,
    pub failure: CloneFailureCode,
    pub residual: CloneResidual,
}

/// Clone's disjoint wire tags cannot be decoded as historical Worktree results.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "result", rename_all = "snake_case")]
pub enum CloneExecutionResult {
    CloneReady(CloneReady),
    CloneFailed(CloneFailed),
}

impl CloneExecutionResult {
    /// Keeps the same result checks on events and Completed query responses.
    pub(crate) fn validate(&self) -> Result<(), MessageValidationError> {
        let (node, spec) = match self {
            Self::CloneReady(result) => (&result.node, &result.spec),
            Self::CloneFailed(result) => (&result.node, &result.spec),
        };
        node.validate()
            .map_err(|field| MessageValidationError::EmptyField { field })?;
        spec.validate()?;
        if node.node_id != spec.node_id {
            return Err(MessageValidationError::CloneTargetMismatch);
        }
        match self {
            Self::CloneReady(result) => {
                validate_destination(&result.repository_id, &result.path)?;
                validate_commit(&result.commit)
                    .map_err(|_| MessageValidationError::InvalidCloneCommit)?;
                // A checkout at any other commit would become every later delivery's base.
                if result.commit != result.spec.base_commit {
                    return Err(MessageValidationError::CloneCommitMismatch);
                }
                Ok(())
            }
            Self::CloneFailed(result) if result.failure == CloneFailureCode::BranchNotFound => {
                Err(MessageValidationError::RetiredCloneFailure)
            }
            Self::CloneFailed(result) => match &result.residual {
                CloneResidual::NoDirectory {} => Ok(()),
                CloneResidual::Retained {
                    repository_id,
                    path,
                } => validate_destination(repository_id, path),
            },
        }
    }

    /// Preserves the original incarnation when a restarted Node reports stored evidence.
    pub(crate) fn node(&self) -> &NodeRuntimeIdentity {
        match self {
            Self::CloneReady(result) => &result.node,
            Self::CloneFailed(result) => &result.node,
        }
    }
}

/// Checks opaque destination facts without interpreting another Node's filesystem locally.
fn validate_destination(
    repository_id: &RepositoryId,
    path: &NodePath,
) -> Result<(), MessageValidationError> {
    if repository_id.is_empty() {
        return Err(MessageValidationError::EmptyField {
            field: "repository_id",
        });
    }
    if path.as_str().trim().is_empty() {
        return Err(MessageValidationError::EmptyField { field: "path" });
    }
    Ok(())
}
