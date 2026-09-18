use ora_node_protocol::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A generated destination is frozen before any directory or Git side effect.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CloneTarget {
    pub repository_id: RepositoryId,
    pub root: PathBuf,
    pub path: PathBuf,
}

/// Filesystem evidence is opaque to SQLite; the filesystem owner verifies its native identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum ClonePhase {
    Reserved,
    DirectoryCreated { identity: String },
    Dispatched { identity: String },
}

/// Unknown retains the last durable phase, never authorizing a new clone attempt.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "evidence", rename_all = "snake_case")]
pub enum CloneProgress {
    Pending(ClonePhase),
    Unknown(ClonePhase),
    Completed(CloneExecutionResult),
}

impl CloneProgress {
    /// Maps retained evidence without treating an interrupted attempt as a terminal failure.
    pub fn state(&self) -> ExecutionState {
        match self {
            Self::Pending(ClonePhase::Reserved) => ExecutionState::Accepted,
            Self::Pending(ClonePhase::DirectoryCreated { .. } | ClonePhase::Dispatched { .. }) => {
                ExecutionState::Running
            }
            Self::Unknown(_) => ExecutionState::Unknown,
            Self::Completed(result) => {
                ExecutionState::Completed(ExecutionResult::Clone(result.clone()))
            }
        }
    }

    /// Supplies the indexed discriminator independently of the serialized business phase.
    pub(crate) fn kind(&self) -> &'static str {
        match self.state() {
            ExecutionState::Accepted => "accepted",
            ExecutionState::Running => "running",
            ExecutionState::Unknown => "unknown",
            ExecutionState::Completed(_) => "completed",
        }
    }
}

/// Durable acquisition responsibility, including the destination retained after failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CloneExecution {
    pub command: CloneRepositoryMessage,
    pub target: CloneTarget,
    pub progress: CloneProgress,
}

impl CloneExecution {
    /// Creates the immutable delivery envelope once, independently of status queries.
    pub fn event(&self, result: CloneExecutionResult) -> NodeToControllerMessage {
        NodeToControllerMessage::CloneResult(CloneResultMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            request_id: self.command.request_id.clone(),
            operation_id: self.command.operation_id.clone(),
            execution_id: self.command.execution_id.clone(),
            sequence: Sequence::new(/*value*/ 1),
            payload: result,
        })
    }
}
