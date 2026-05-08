use crate::{ArtifactId, AuditFields, TaskId};
use serde::{Deserialize, Serialize};

/// Represents a persisted artifact that belongs to a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    pub id: ArtifactId,
    pub task_id: TaskId,
    pub content: Option<String>,
    pub audit_fields: AuditFields,
}

impl Artifact {
    /// Creates an artifact snapshot together with its persistence-managed audit metadata.
    pub fn new(
        id: ArtifactId,
        task_id: TaskId,
        content: Option<String>,
        audit_fields: AuditFields,
    ) -> Self {
        Self {
            id,
            task_id,
            content,
            audit_fields,
        }
    }
}
