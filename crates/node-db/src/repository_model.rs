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
    DirectoryCreated {
        identity: String,
    },
    /// The Node fetches the snapshot into its transport repository. `attempts` counts the
    /// passes started, recorded before each starts: a restart discards the transport repository
    /// and starts another pass, and the count bounds how often that can happen.
    Fetching {
        identity: String,
        attempts: u32,
    },
    /// The workload command `step` of the fixed checkout sequence is recorded or running; the
    /// commands before it exited successfully.
    Dispatched {
        identity: String,
        step: u32,
    },
}

impl ClonePhase {
    /// The created directory's recorded identity, once the execution got that far.
    pub fn identity(&self) -> Option<&str> {
        match self {
            Self::Reserved => None,
            Self::DirectoryCreated { identity }
            | Self::Fetching { identity, .. }
            | Self::Dispatched { identity, .. } => Some(identity),
        }
    }
}

/// Unknown retains the last durable phase, never authorizing a new clone attempt.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "evidence", rename_all = "snake_case")]
pub enum CloneProgress {
    Pending(ClonePhase),
    Unknown(ClonePhase),
    // Boxed: the result repeats the snapshot input, which would inflate every pending record.
    Completed(Box<CloneExecutionResult>),
}

impl CloneProgress {
    /// Maps retained evidence without treating an interrupted attempt as a terminal failure.
    pub fn state(&self) -> ExecutionState {
        match self {
            Self::Pending(ClonePhase::Reserved) => ExecutionState::Accepted,
            Self::Pending(
                ClonePhase::DirectoryCreated { .. }
                | ClonePhase::Fetching { .. }
                | ClonePhase::Dispatched { .. },
            ) => ExecutionState::Running,
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

/// A successful clone's checkout as session, delivery and `ora-git` use it: where it is and the
/// snapshot input it was made from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClonedCheckout {
    pub path: PathBuf,
    pub spec: CloneExecutionSpec,
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
