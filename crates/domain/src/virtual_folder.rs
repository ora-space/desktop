use crate::{AuditFields, ProjectId};
use serde::{Deserialize, Serialize};

/// Represents a metadata-driven folder mounted into a project workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualFolder {
    pub id: crate::VirtualFolderId,
    pub project_id: ProjectId,
    pub name: String,
    pub mount_point: String,
    pub audit_fields: AuditFields,
}

impl VirtualFolder {
    /// Creates a virtual folder snapshot together with its persistence-managed audit metadata.
    pub fn new(
        id: crate::VirtualFolderId,
        project_id: ProjectId,
        name: impl Into<String>,
        mount_point: impl Into<String>,
        audit_fields: AuditFields,
    ) -> Self {
        Self {
            id,
            project_id,
            name: name.into(),
            mount_point: mount_point.into(),
            audit_fields,
        }
    }
}
