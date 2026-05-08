use crate::{AuditFields, ProjectId};
use serde::{Deserialize, Serialize};

/// Represents a top-level Ora project rooted at a physical workspace path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub root_path: String,
    pub audit_fields: AuditFields,
}

impl Project {
    /// Creates a project snapshot together with its persistence-managed audit metadata.
    pub fn new(
        id: ProjectId,
        name: impl Into<String>,
        root_path: impl Into<String>,
        audit_fields: AuditFields,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            root_path: root_path.into(),
            audit_fields,
        }
    }
}
