use crate::{
    AgentSessionResult, CloneExecutionResult, MessageValidationError, NodeRuntimeIdentity,
    PluginExecutionResult, RevisionExecutionResult, WorktreeExecutionResult,
};
use serde::{Deserialize, Serialize};

/// Business-owned terminal evidence shared by status queries, without changing stored Worktree results.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ExecutionResult {
    // Each business owns disjoint `kind` tags. Keeping these encodings avoids rewriting
    // historical Worktree evidence just to introduce a second result family.
    Worktree(WorktreeExecutionResult),
    // Boxed because the snapshot input a clone result repeats would otherwise inflate every
    // ExecutionState a status query builds.
    Clone(Box<CloneExecutionResult>),
    Plugin(PluginExecutionResult),
    AgentSession(AgentSessionResult),
    // Boxed because a delivered Revision carries two object descriptions and three commits,
    // which would otherwise inflate every ExecutionState a status query builds.
    Revision(Box<RevisionExecutionResult>),
}

impl ExecutionResult {
    /// Delegates concrete invariants to the business that produced the evidence.
    pub(crate) fn validate(&self) -> Result<(), MessageValidationError> {
        match self {
            Self::Worktree(result) => result
                .validate()
                .map_err(|field| MessageValidationError::EmptyField { field }),
            Self::Clone(result) => result.validate(),
            Self::Plugin(result) => result.validate(),
            Self::AgentSession(result) => result.validate(),
            Self::Revision(result) => result.validate(),
        }
    }

    /// Exposes origin without requiring execution status to understand business variants.
    pub(crate) fn node(&self) -> &NodeRuntimeIdentity {
        match self {
            Self::Worktree(result) => result.node(),
            Self::Clone(result) => result.node(),
            Self::Plugin(result) => result.node(),
            Self::AgentSession(result) => result.node(),
            Self::Revision(result) => result.node(),
        }
    }
}
