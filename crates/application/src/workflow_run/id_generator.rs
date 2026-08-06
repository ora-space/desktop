use ora_domain::WorkflowRunId;
use uuid::Uuid;

use super::ports::WorkflowRunIdGenerator;

/// Generates UUID-based identifiers for workflow runs.
#[derive(Clone, Debug, Default)]
pub struct UuidWorkflowRunIdGenerator;

impl UuidWorkflowRunIdGenerator {
    /// Creates a new UUID v4-based identifier generator.
    pub fn new() -> Self {
        Self
    }
}

impl WorkflowRunIdGenerator for UuidWorkflowRunIdGenerator {
    fn generate_run_id(&self) -> WorkflowRunId {
        WorkflowRunId::new(Uuid::new_v4().to_string())
    }
}
