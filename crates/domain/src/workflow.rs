use crate::{AuditFields, DomainModelError, Namespace, WorkflowId, WorkflowSnapshotId};
use serde::{Deserialize, Serialize};

/// Represents one workflow definition entity with its lifecycle metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workflow {
    pub id: WorkflowId,
    pub namespace: Namespace,
    pub name: String,
    pub published_snapshot_id: Option<WorkflowSnapshotId>,
    pub audit_fields: AuditFields,
}

impl Workflow {
    /// Creates a workflow while normalizing its user-facing name for stable lookup.
    pub fn new(
        id: WorkflowId,
        namespace: Namespace,
        name: impl Into<String>,
        published_snapshot_id: Option<WorkflowSnapshotId>,
        audit_fields: AuditFields,
    ) -> Result<Self, DomainModelError> {
        let name = name.into().trim().to_string();

        if name.is_empty() {
            return Err(DomainModelError::EmptyWorkflowName);
        }

        Ok(Self {
            id,
            namespace,
            name,
            published_snapshot_id,
            audit_fields,
        })
    }
}

/// Represents one versioned snapshot of a workflow graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowSnapshot {
    pub id: WorkflowSnapshotId,
    pub workflow_id: WorkflowId,
    pub version: String,
    pub graph: String,
    pub created_at: i64,
    pub updated_at: Option<i64>,
    pub is_deleted: bool,
}

impl WorkflowSnapshot {
    /// Creates a snapshot from already-validated version and graph fields.
    pub fn new(
        id: WorkflowSnapshotId,
        workflow_id: WorkflowId,
        version: impl Into<String>,
        graph: impl Into<String>,
        created_at: i64,
        updated_at: Option<i64>,
        is_deleted: bool,
    ) -> Self {
        Self {
            id,
            workflow_id,
            version: version.into(),
            graph: graph.into(),
            created_at,
            updated_at,
            is_deleted,
        }
    }
}

// ── Read models ──

/// Returned after a workflow is created with its initial draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedWorkflow {
    pub workflow: Workflow,
    pub draft: WorkflowSnapshot,
}

/// Full workflow detail including draft and currently published snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowDetail {
    pub workflow: Workflow,
    pub draft: WorkflowSnapshot,
    pub published: Option<WorkflowSnapshot>,
}

/// Lightweight workflow summary for list views — no graph data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowSummary {
    pub id: String,
    pub namespace: Namespace,
    pub name: String,
    pub published_version: Option<String>,
    /// Canonical `<namespace>/<identifier>` plugin id that imported this workflow, if any.
    ///
    /// Derived purely from marketplace import records for informational source status; does not
    /// grant ownership or alter the workflow's `local` namespace.
    pub source_plugin_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Version metadata without graph content, used in version history listings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowVersion {
    pub id: String,
    pub version: String,
    pub created_at: i64,
}
