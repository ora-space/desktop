use crate::{AuditFields, ProjectId};
use serde::{Deserialize, Serialize};

/// Represents a logical Ora project; checkout identity lives on its workspaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub repository_kind: String,
    pub repository_url: Option<String>,
    pub default_branch: Option<String>,
    pub audit_fields: AuditFields,
}

impl Project {
    /// Creates a project snapshot together with its persistence-managed audit metadata.
    pub fn new(id: ProjectId, name: impl Into<String>, audit_fields: AuditFields) -> Self {
        Self {
            id,
            name: name.into(),
            repository_kind: "git".to_string(),
            repository_url: None,
            default_branch: None,
            audit_fields,
        }
    }
}
