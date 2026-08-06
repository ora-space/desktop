use crate::{AuditFields, ProjectId, ProjectSpecSourceOverrideId};
use serde::{Deserialize, Serialize};

/// Identifies the workflow semantics attached to a project specification directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpecWorkflow {
    OpenSpec,
    Superpowers,
    Custom { name: String },
}

/// Controls whether an override enables or suppresses a specification source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpecSourceVisibility {
    Enabled,
    Disabled,
}

/// Persists one project-wide source decision shared by the root checkout and every worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSpecSourceOverride {
    pub id: ProjectSpecSourceOverrideId,
    pub project_id: ProjectId,
    pub relative_path: String,
    pub workflow: SpecWorkflow,
    pub visibility: SpecSourceVisibility,
    pub audit_fields: AuditFields,
}

impl ProjectSpecSourceOverride {
    /// Creates a validated application-owned snapshot ready for transactional replacement.
    pub fn new(
        id: ProjectSpecSourceOverrideId,
        project_id: ProjectId,
        relative_path: impl Into<String>,
        workflow: SpecWorkflow,
        visibility: SpecSourceVisibility,
        audit_fields: AuditFields,
    ) -> Self {
        Self {
            id,
            project_id,
            relative_path: relative_path.into(),
            workflow,
            visibility,
            audit_fields,
        }
    }
}
