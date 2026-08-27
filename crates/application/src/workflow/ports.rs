use crate::RepositoryError;
use ora_domain::{
    CreatedWorkflow, Namespace, Workflow, WorkflowDetail, WorkflowId, WorkflowSnapshot,
    WorkflowSnapshotId, WorkflowSummary, WorkflowVersion,
};

/// Describes the outcome of deleting a snapshot while preserving aggregate invariants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteSnapshotResult {
    Deleted(WorkflowSnapshot),
    WorkflowNotFound,
    SnapshotNotFound,
    DraftSnapshot,
    ActiveSnapshot,
    SnapshotInUse,
}

/// Describes the outcome of deleting a workflow while preserving aggregate invariants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteWorkflowResult {
    Deleted,
    NotFound,
    ActiveRuns,
}

/// Describes the outcome of replacing a workflow's editable fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateWorkflowResult {
    Updated(Workflow),
    WorkflowNotFound,
}

/// Describes the outcome of updating a workflow draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateDraftResult {
    Updated(WorkflowSnapshot),
    WorkflowNotFound,
    DraftNotFound,
}

/// Describes the outcome of publishing a draft as an immutable snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishSnapshotResult {
    Published(WorkflowSnapshot),
    WorkflowNotFound,
    DraftNotFound,
    VersionAlreadyExists,
}

/// Describes the outcome of copying a historical snapshot into the draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RollbackDraftResult {
    DraftUpdated(WorkflowSnapshot),
    WorkflowNotFound,
    SnapshotNotFound,
    DraftSnapshot,
    DraftNotFound,
}

/// Describes the outcome of activating a historical snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivateVersionResult {
    Activated(WorkflowSnapshot),
    WorkflowNotFound,
    SnapshotNotFound,
    DraftSnapshot,
    DraftNotFound,
}

/// Defines persistence operations for the workflow aggregate.
///
/// Methods represent domain operations rather than individual SQL statements,
/// so create, publish, rollback, and activate each execute within a single
/// repository-managed transaction.
pub trait WorkflowRepository {
    /// Persists a new workflow together with its initial draft in one transaction.
    fn create_workflow(
        &self,
        workflow: Workflow,
        draft: WorkflowSnapshot,
    ) -> Result<CreatedWorkflow, RepositoryError>;

    /// Loads one visible workflow by identifier.
    fn find_workflow(&self, workflow_id: &WorkflowId) -> Result<Option<Workflow>, RepositoryError>;

    /// Loads one visible workflow by namespace and ASCII case-insensitive name.
    fn find_workflow_by_name(
        &self,
        namespace: &Namespace,
        name: &str,
    ) -> Result<Option<Workflow>, RepositoryError>;

    /// Loads a workflow together with its draft and currently published snapshot.
    fn get_workflow_detail(
        &self,
        workflow_id: &WorkflowId,
    ) -> Result<Option<WorkflowDetail>, RepositoryError>;

    /// Lists visible workflows with their published version, newest created first.
    fn list_workflows(&self) -> Result<Vec<WorkflowSummary>, RepositoryError>;

    /// Replaces a visible workflow's editable name and returns the authoritative stored aggregate.
    fn update_workflow(
        &self,
        workflow_id: &WorkflowId,
        name: String,
        updated_at: i64,
    ) -> Result<UpdateWorkflowResult, RepositoryError>;

    /// Marks a visible workflow deleted and cascades the soft-delete to all its snapshots
    /// within a single transaction, after refusing workflows whose snapshots a live run freezes.
    fn soft_delete_workflow(
        &self,
        workflow_id: &WorkflowId,
        deleted_at: i64,
    ) -> Result<DeleteWorkflowResult, RepositoryError>;

    /// Loads one visible snapshot by workflow and version string (works for both `"draft"`
    /// and published version identifiers).
    fn find_snapshot_by_version(
        &self,
        workflow_id: &WorkflowId,
        version: &str,
    ) -> Result<Option<WorkflowSnapshot>, RepositoryError>;

    /// Loads one visible snapshot by identifier, constrained to the owning workflow so a caller
    /// can distinguish "missing or not in this workflow" from a snapshot of another workflow.
    fn find_snapshot_by_id(
        &self,
        workflow_id: &WorkflowId,
        snapshot_id: &WorkflowSnapshotId,
    ) -> Result<Option<WorkflowSnapshot>, RepositoryError>;

    /// Loads one visible snapshot by identifier alone, independent of its owning workflow.
    ///
    /// Snapshot ids are globally unique, so a caller holding only a run's frozen snapshot id can
    /// resolve its graph without first knowing the workflow.
    fn find_snapshot_any_workflow(
        &self,
        snapshot_id: &WorkflowSnapshotId,
    ) -> Result<Option<WorkflowSnapshot>, RepositoryError>;

    /// Lists published (non-draft, non-deleted) version summaries for a workflow,
    /// ordered by creation time descending.
    fn list_versions(
        &self,
        workflow_id: &WorkflowId,
    ) -> Result<Vec<WorkflowVersion>, RepositoryError>;

    /// Updates the graph of a workflow's draft snapshot in-place.
    fn update_draft(
        &self,
        workflow_id: &WorkflowId,
        graph: String,
        updated_at: i64,
    ) -> Result<UpdateDraftResult, RepositoryError>;

    /// Publishes the current draft as an immutable snapshot and activates it
    /// (sets `published_snapshot_id`) within a single transaction.
    fn publish_snapshot(
        &self,
        workflow_id: &WorkflowId,
        snapshot_id: WorkflowSnapshotId,
        version: String,
        created_at: i64,
    ) -> Result<PublishSnapshotResult, RepositoryError>;

    /// Copies the graph from a historical snapshot into the draft without changing
    /// the published version pointer.
    fn rollback_draft(
        &self,
        workflow_id: &WorkflowId,
        snapshot_id: &WorkflowSnapshotId,
        updated_at: i64,
    ) -> Result<RollbackDraftResult, RepositoryError>;

    /// Switches the published version pointer to a different snapshot and syncs its
    /// graph into the draft within a single transaction.
    fn activate_version(
        &self,
        workflow_id: &WorkflowId,
        snapshot_id: &WorkflowSnapshotId,
        updated_at: i64,
    ) -> Result<ActivateVersionResult, RepositoryError>;

    /// Marks a visible non-draft, non-active snapshot deleted.
    fn soft_delete_snapshot(
        &self,
        workflow_id: &WorkflowId,
        snapshot_id: &WorkflowSnapshotId,
        deleted_at: i64,
    ) -> Result<DeleteSnapshotResult, RepositoryError>;
}

/// Supplies new workflow and snapshot identifiers for create use cases.
pub trait WorkflowIdGenerator {
    /// Produces the identifier for a newly created workflow.
    fn generate_workflow_id(&self) -> WorkflowId;

    /// Produces the identifier for a newly created snapshot.
    fn generate_snapshot_id(&self) -> WorkflowSnapshotId;
}
