use crate::{
    CloneExecutionResult, MessageValidationError, NodeRuntimeIdentity, WorktreeExecutionResult,
};
use serde::{Deserialize, Serialize};

/// Business-owned terminal evidence shared by status queries, without changing stored Worktree results.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ExecutionResult {
    // Each business owns disjoint `kind` tags. Keeping these encodings avoids rewriting
    // historical Worktree evidence just to introduce a second result family.
    Worktree(WorktreeExecutionResult),
    Clone(CloneExecutionResult),
}

impl ExecutionResult {
    /// Delegates concrete invariants to the business that produced the evidence.
    pub(crate) fn validate(&self) -> Result<(), MessageValidationError> {
        match self {
            Self::Worktree(result) => result
                .validate()
                .map_err(|field| MessageValidationError::EmptyField { field }),
            Self::Clone(result) => result.validate(),
        }
    }

    /// Exposes origin without requiring execution status to understand business variants.
    pub(crate) fn node(&self) -> &NodeRuntimeIdentity {
        match self {
            Self::Worktree(result) => result.node(),
            Self::Clone(result) => result.node(),
        }
    }
}
