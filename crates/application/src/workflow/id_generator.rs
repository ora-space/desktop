use ora_domain::{WorkflowId, WorkflowSnapshotId};
use uuid::Uuid;

use super::ports::WorkflowIdGenerator;

/// Generates UUID-based identifiers for workflows and snapshots.
#[derive(Clone, Debug, Default)]
pub struct UuidWorkflowIdGenerator;

impl UuidWorkflowIdGenerator {
    /// Creates a new UUID v4-based identifier generator.
    pub fn new() -> Self {
        Self
    }
}

impl WorkflowIdGenerator for UuidWorkflowIdGenerator {
    fn generate_workflow_id(&self) -> WorkflowId {
        WorkflowId::new(Uuid::new_v4().to_string())
    }

    fn generate_snapshot_id(&self) -> WorkflowSnapshotId {
        WorkflowSnapshotId::new(Uuid::new_v4().to_string())
    }
}
