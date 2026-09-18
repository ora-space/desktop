use ora_node_protocol::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Typed commands retain the complete input separately from resolved Git facts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub enum Command {
    Ensure(EnsureWorktreeMessage),
    Remove(RemoveWorktreeMessage),
}
impl Command {
    /// Returns the immutable operation identity used for durable deduplication.
    pub fn operation_id(&self) -> &OperationId {
        match self {
            Self::Ensure(m) => &m.operation_id,
            Self::Remove(m) => &m.operation_id,
        }
    }
    /// Returns the execution identity that cannot be rebound to another operation.
    pub fn execution_id(&self) -> &ExecutionId {
        match self {
            Self::Ensure(m) => &m.execution_id,
            Self::Remove(m) => &m.execution_id,
        }
    }
    /// Returns resource intent without interpreting opaque identities as filesystem paths.
    pub fn spec(&self) -> &WorktreeExecutionSpec {
        match self {
            Self::Ensure(m) => &m.payload.spec,
            Self::Remove(m) => &m.payload.spec,
        }
    }
    /// Reuses wire semantic validation for in-process callers.
    pub fn validate(&self) -> Result<(), MessageValidationError> {
        match self {
            Self::Ensure(m) => ControllerToNodeMessage::EnsureWorktree(m.clone()).validate(),
            Self::Remove(m) => ControllerToNodeMessage::RemoveWorktree(m.clone()).validate(),
        }
    }
    /// Builds the original terminal envelope once; replay never regenerates metadata.
    pub fn event(&self, result: WorktreeExecutionResult) -> NodeToControllerMessage {
        let protocol_version = CURRENT_PROTOCOL_VERSION;
        let request_id = match self {
            Self::Ensure(m) => m.request_id.clone(),
            Self::Remove(m) => m.request_id.clone(),
        };
        let operation_id = self.operation_id().clone();
        let execution_id = self.execution_id().clone();
        let sequence = Sequence::new(/*value*/ 1);
        match result {
            WorktreeExecutionResult::Ready(payload) => {
                NodeToControllerMessage::WorktreeReady(WorktreeReadyMessage {
                    protocol_version,
                    request_id,
                    operation_id,
                    execution_id,
                    sequence,
                    payload,
                })
            }
            WorktreeExecutionResult::Failed(payload) => {
                NodeToControllerMessage::WorktreeFailed(WorktreeFailedMessage {
                    protocol_version,
                    request_id,
                    operation_id,
                    execution_id,
                    sequence,
                    payload,
                })
            }
            WorktreeExecutionResult::Removed(payload) => {
                NodeToControllerMessage::WorktreeRemoved(WorktreeRemovedMessage {
                    protocol_version,
                    request_id,
                    operation_id,
                    execution_id,
                    sequence,
                    payload,
                })
            }
            WorktreeExecutionResult::RemovalFailed(payload) => {
                NodeToControllerMessage::WorktreeRemovalFailed(WorktreeRemovalFailedMessage {
                    protocol_version,
                    request_id,
                    operation_id,
                    execution_id,
                    sequence,
                    payload,
                })
            }
        }
    }
}

/// Frozen binding and immutable baseline used for execution and all subsequent recovery.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub main_path: PathBuf,
    pub git_directory: PathBuf,
    pub authorized_root: PathBuf,
    pub worktree_root: PathBuf,
    pub path: PathBuf,
    pub branch: BranchName,
    pub base_commit: CommitId,
}

/// Progress is persisted before each independently recoverable external mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stage {
    Create,
    CleanupCreation,
    RemoveWorktree,
    RemoveBranch,
}

/// Terminal evidence is a variant, so incomplete records cannot carry a terminal result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Progress {
    Accepted,
    Running {
        stage: Stage,
        observer: NodeRuntimeIdentity,
        observed_at: String,
    },
    Unknown {
        stage: Stage,
        observer: NodeRuntimeIdentity,
        observed_at: String,
        diagnostic: String,
    },
    Completed {
        result: WorktreeExecutionResult,
    },
}
impl Progress {
    /// Maps retained evidence to the protocol without turning unknown results into failures.
    pub fn state(&self) -> ExecutionState {
        match self {
            Self::Accepted => ExecutionState::Accepted,
            Self::Running { .. } => ExecutionState::Running,
            Self::Unknown { .. } => ExecutionState::Unknown,
            Self::Completed { result } => ExecutionState::Completed(
                ora_node_protocol::ExecutionResult::Worktree(result.clone()),
            ),
        }
    }
    /// Supplies the constrained SQL discriminator for state scans and guarded transitions.
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Running { .. } => "running",
            Self::Unknown { .. } => "unknown",
            Self::Completed { .. } => "completed",
        }
    }
}

/// The full durable execution evidence returned through behavior-oriented storage APIs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Execution {
    pub command: Command,
    pub target: Option<Target>,
    pub progress: Progress,
}

/// Resource tombstones retain ownership evidence even after successful removal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resource {
    pub spec: WorktreeExecutionSpec,
    pub target: Target,
    pub state: ResourceState,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceState {
    Reserved,
    Present,
    Removed,
}

/// Testable persistence boundaries; implementations may refuse a write before it commits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WritePoint {
    Process,
    Accept,
    Progress,
    Complete,
    Outbox,
    Acknowledge,
}
/// Injects storage failure without substituting an in-memory database for durable tests.
/// Implementations must return an error before the named transaction can commit.
pub trait WriteGuard {
    /// Refuses the named boundary before its transaction commits.
    fn before_write(&self, point: WritePoint) -> Result<(), crate::Error>;
}
/// Production writes rely on SQLite's actual error reporting.
pub struct DurableWrites;
impl WriteGuard for DurableWrites {
    /// Allows SQLite to perform and report the write.
    fn before_write(&self, _point: WritePoint) -> Result<(), crate::Error> {
        Ok(())
    }
}
