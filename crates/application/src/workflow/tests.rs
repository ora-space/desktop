use std::sync::{Arc, Mutex};

use ora_domain::{
    CreatedWorkflow, Workflow, WorkflowDetail, WorkflowId, WorkflowSnapshot, WorkflowSnapshotId,
    WorkflowSummary, WorkflowVersion,
};
use pretty_assertions::assert_eq;

use super::{
    ActivateVersionResult, DeleteSnapshotResult, GetWorkflowSnapshotHandler, PublishSnapshotResult,
    PublishWorkflowHandler, RollbackDraftResult, UpdateDraftResult, UpdateWorkflowResult,
    WorkflowIdGenerator, WorkflowRepository,
};
use crate::workflow::mapper::map_snapshot;
use crate::{ApplicationError, Clock, RepositoryError};

/// Verifies automatic versions derive from the injected clock used for snapshot timestamps.
#[test]
fn publish_uses_the_injected_clock_for_automatic_versions() {
    let handler = PublishWorkflowHandler::new(
        Arc::new(PublishRepository::new(draft_snapshot(), Vec::new())),
        FixedWorkflowIdGenerator,
        FixedClock(42),
    );

    let response = handler
        .handle(ora_contracts::PublishWorkflowRequest {
            workflow_id: "workflow-1".to_string(),
            version: None,
        })
        .unwrap();

    assert_eq!(
        response.snapshot,
        ora_contracts::WorkflowSnapshot {
            id: "snapshot-1".to_string(),
            workflow_id: "workflow-1".to_string(),
            version: "v42".to_string(),
            graph: "{\"nodes\":[]}".to_string(),
            created_at: 42,
            updated_at: None,
        }
    );
}

/// Verifies automatic versions retry with a stable suffix when the injected clock collides.
#[test]
fn publish_retries_automatic_versions_that_collide_at_the_same_clock_value() {
    let handler = PublishWorkflowHandler::new(
        Arc::new(PublishRepository::new(
            draft_snapshot(),
            vec!["v42".to_string()],
        )),
        FixedWorkflowIdGenerator,
        FixedClock(42),
    );

    let response = handler
        .handle(ora_contracts::PublishWorkflowRequest {
            workflow_id: "workflow-1".to_string(),
            version: None,
        })
        .unwrap();

    assert_eq!(
        response.snapshot,
        ora_contracts::WorkflowSnapshot {
            id: "snapshot-1".to_string(),
            workflow_id: "workflow-1".to_string(),
            version: "v42-1".to_string(),
            graph: "{\"nodes\":[]}".to_string(),
            created_at: 42,
            updated_at: None,
        }
    );
}

/// Verifies versions that cannot be represented as a single URL path segment are rejected.
#[test]
fn publish_rejects_an_invalid_version_before_writing() {
    let handler = PublishWorkflowHandler::new(
        Arc::new(PublishRepository::new(draft_snapshot(), Vec::new())),
        FixedWorkflowIdGenerator,
        FixedClock(42),
    );

    for version in ["release/1", ".", ".."] {
        assert_eq!(
            handler
                .handle(ora_contracts::PublishWorkflowRequest {
                    workflow_id: "workflow-1".to_string(),
                    version: Some(version.to_string()),
                })
                .unwrap_err(),
            ApplicationError::WorkflowVersionInvalid
        );
    }
}

/// Supplies the fixed draft needed by publish-handler tests.
#[derive(Debug)]
struct PublishRepository {
    draft: WorkflowSnapshot,
    occupied_versions: Mutex<Vec<String>>,
}

impl PublishRepository {
    /// Builds the publish-specific repository fake around one visible draft.
    fn new(draft: WorkflowSnapshot, occupied_versions: Vec<String>) -> Self {
        Self {
            draft,
            occupied_versions: Mutex::new(occupied_versions),
        }
    }
}

impl WorkflowRepository for PublishRepository {
    fn create_workflow(
        &self,
        _workflow: Workflow,
        _draft: WorkflowSnapshot,
    ) -> Result<CreatedWorkflow, RepositoryError> {
        unreachable!("publish tests never create workflows")
    }

    fn find_workflow(
        &self,
        _workflow_id: &WorkflowId,
    ) -> Result<Option<Workflow>, RepositoryError> {
        unreachable!("publish tests never load workflows")
    }

    fn get_workflow_detail(
        &self,
        _workflow_id: &WorkflowId,
    ) -> Result<Option<WorkflowDetail>, RepositoryError> {
        unreachable!("publish tests never load workflow details")
    }

    fn list_workflows(&self) -> Result<Vec<WorkflowSummary>, RepositoryError> {
        unreachable!("publish tests never list workflows")
    }

    fn update_workflow(
        &self,
        _workflow_id: &WorkflowId,
        _name: String,
        _updated_at: i64,
    ) -> Result<UpdateWorkflowResult, RepositoryError> {
        unreachable!("publish tests never update workflows")
    }

    fn soft_delete_workflow(
        &self,
        _workflow_id: &WorkflowId,
        _deleted_at: i64,
    ) -> Result<crate::DeleteWorkflowResult, RepositoryError> {
        unreachable!("publish tests never delete workflows")
    }

    fn find_snapshot_by_version(
        &self,
        _workflow_id: &WorkflowId,
        version: &str,
    ) -> Result<Option<WorkflowSnapshot>, RepositoryError> {
        assert_eq!(version, "draft");
        Ok(Some(self.draft.clone()))
    }

    fn list_versions(
        &self,
        _workflow_id: &WorkflowId,
    ) -> Result<Vec<WorkflowVersion>, RepositoryError> {
        unreachable!("publish tests never list versions")
    }

    fn update_draft(
        &self,
        _workflow_id: &WorkflowId,
        _graph: String,
        _updated_at: i64,
    ) -> Result<UpdateDraftResult, RepositoryError> {
        unreachable!("publish tests never update drafts")
    }

    fn publish_snapshot(
        &self,
        _workflow_id: &WorkflowId,
        snapshot_id: WorkflowSnapshotId,
        version: String,
        created_at: i64,
    ) -> Result<PublishSnapshotResult, RepositoryError> {
        let mut occupied_versions = self.occupied_versions.lock().unwrap();
        if occupied_versions.contains(&version) {
            return Ok(PublishSnapshotResult::VersionAlreadyExists);
        }
        occupied_versions.push(version.clone());

        Ok(PublishSnapshotResult::Published(WorkflowSnapshot::new(
            snapshot_id,
            self.draft.workflow_id.clone(),
            version,
            self.draft.graph.clone(),
            created_at,
            /*updated_at*/ None,
            /*is_deleted*/ false,
        )))
    }

    fn rollback_draft(
        &self,
        _workflow_id: &WorkflowId,
        _snapshot_id: &WorkflowSnapshotId,
        _updated_at: i64,
    ) -> Result<RollbackDraftResult, RepositoryError> {
        unreachable!("publish tests never roll back drafts")
    }

    fn activate_version(
        &self,
        _workflow_id: &WorkflowId,
        _snapshot_id: &WorkflowSnapshotId,
        _updated_at: i64,
    ) -> Result<ActivateVersionResult, RepositoryError> {
        unreachable!("publish tests never activate versions")
    }

    fn soft_delete_snapshot(
        &self,
        _workflow_id: &WorkflowId,
        _snapshot_id: &WorkflowSnapshotId,
        _deleted_at: i64,
    ) -> Result<DeleteSnapshotResult, RepositoryError> {
        unreachable!("publish tests never delete snapshots")
    }

    fn find_snapshot_by_id(
        &self,
        _workflow_id: &WorkflowId,
        _snapshot_id: &WorkflowSnapshotId,
    ) -> Result<Option<WorkflowSnapshot>, RepositoryError> {
        unreachable!("publish tests never resolve snapshots by id")
    }

    fn find_snapshot_any_workflow(
        &self,
        _snapshot_id: &WorkflowSnapshotId,
    ) -> Result<Option<WorkflowSnapshot>, RepositoryError> {
        unreachable!("publish tests never resolve snapshots by id")
    }
}

/// Returns the draft copied by publish-handler tests.
fn draft_snapshot() -> WorkflowSnapshot {
    WorkflowSnapshot::new(
        WorkflowSnapshotId::new("draft-1"),
        WorkflowId::new("workflow-1"),
        "draft",
        "{\"nodes\":[]}",
        1,
        Some(1),
        /*is_deleted*/ false,
    )
}

/// Produces deterministic identifiers for publish-handler tests.
#[derive(Clone, Copy, Debug)]
struct FixedWorkflowIdGenerator;

impl WorkflowIdGenerator for FixedWorkflowIdGenerator {
    fn generate_workflow_id(&self) -> WorkflowId {
        WorkflowId::new("workflow-1")
    }

    fn generate_snapshot_id(&self) -> WorkflowSnapshotId {
        WorkflowSnapshotId::new("snapshot-1")
    }
}

/// Supplies deterministic timestamps to workflow handlers.
#[derive(Clone, Copy, Debug)]
struct FixedClock(i64);

impl Clock for FixedClock {
    fn now_timestamp_millis(&self) -> i64 {
        self.0
    }
}

/// Supplies a fixed snapshot (or none) for snapshot-by-id handler tests.
#[derive(Debug)]
struct SnapshotLookupRepository {
    snapshot: Option<WorkflowSnapshot>,
}

impl SnapshotLookupRepository {
    /// Builds the lookup fake around one resolvable snapshot.
    fn found(snapshot: WorkflowSnapshot) -> Self {
        Self {
            snapshot: Some(snapshot),
        }
    }

    /// Builds the lookup fake that resolves no snapshot.
    fn missing() -> Self {
        Self { snapshot: None }
    }
}

impl WorkflowRepository for SnapshotLookupRepository {
    fn create_workflow(
        &self,
        _workflow: Workflow,
        _draft: WorkflowSnapshot,
    ) -> Result<CreatedWorkflow, RepositoryError> {
        unreachable!("snapshot lookup tests never create workflows")
    }

    fn find_workflow(
        &self,
        _workflow_id: &WorkflowId,
    ) -> Result<Option<Workflow>, RepositoryError> {
        unreachable!("snapshot lookup tests never load workflows")
    }

    fn get_workflow_detail(
        &self,
        _workflow_id: &WorkflowId,
    ) -> Result<Option<WorkflowDetail>, RepositoryError> {
        unreachable!("snapshot lookup tests never load workflow details")
    }

    fn list_workflows(&self) -> Result<Vec<WorkflowSummary>, RepositoryError> {
        unreachable!("snapshot lookup tests never list workflows")
    }

    fn update_workflow(
        &self,
        _workflow_id: &WorkflowId,
        _name: String,
        _updated_at: i64,
    ) -> Result<UpdateWorkflowResult, RepositoryError> {
        unreachable!("snapshot lookup tests never update workflows")
    }

    fn soft_delete_workflow(
        &self,
        _workflow_id: &WorkflowId,
        _deleted_at: i64,
    ) -> Result<crate::DeleteWorkflowResult, RepositoryError> {
        unreachable!("snapshot lookup tests never delete workflows")
    }

    fn find_snapshot_by_version(
        &self,
        _workflow_id: &WorkflowId,
        _version: &str,
    ) -> Result<Option<WorkflowSnapshot>, RepositoryError> {
        unreachable!("snapshot lookup tests never resolve snapshots by version")
    }

    fn find_snapshot_by_id(
        &self,
        _workflow_id: &WorkflowId,
        _snapshot_id: &WorkflowSnapshotId,
    ) -> Result<Option<WorkflowSnapshot>, RepositoryError> {
        unreachable!("snapshot lookup tests never resolve scoped snapshots")
    }

    fn find_snapshot_any_workflow(
        &self,
        _snapshot_id: &WorkflowSnapshotId,
    ) -> Result<Option<WorkflowSnapshot>, RepositoryError> {
        Ok(self.snapshot.clone())
    }

    fn list_versions(
        &self,
        _workflow_id: &WorkflowId,
    ) -> Result<Vec<WorkflowVersion>, RepositoryError> {
        unreachable!("snapshot lookup tests never list versions")
    }

    fn update_draft(
        &self,
        _workflow_id: &WorkflowId,
        _graph: String,
        _updated_at: i64,
    ) -> Result<UpdateDraftResult, RepositoryError> {
        unreachable!("snapshot lookup tests never update drafts")
    }

    fn publish_snapshot(
        &self,
        _workflow_id: &WorkflowId,
        _snapshot_id: WorkflowSnapshotId,
        _version: String,
        _created_at: i64,
    ) -> Result<PublishSnapshotResult, RepositoryError> {
        unreachable!("snapshot lookup tests never publish snapshots")
    }

    fn rollback_draft(
        &self,
        _workflow_id: &WorkflowId,
        _snapshot_id: &WorkflowSnapshotId,
        _updated_at: i64,
    ) -> Result<RollbackDraftResult, RepositoryError> {
        unreachable!("snapshot lookup tests never roll back drafts")
    }

    fn activate_version(
        &self,
        _workflow_id: &WorkflowId,
        _snapshot_id: &WorkflowSnapshotId,
        _updated_at: i64,
    ) -> Result<ActivateVersionResult, RepositoryError> {
        unreachable!("snapshot lookup tests never activate versions")
    }

    fn soft_delete_snapshot(
        &self,
        _workflow_id: &WorkflowId,
        _snapshot_id: &WorkflowSnapshotId,
        _deleted_at: i64,
    ) -> Result<DeleteSnapshotResult, RepositoryError> {
        unreachable!("snapshot lookup tests never delete snapshots")
    }
}

/// Verifies the snapshot-by-id handler returns the frozen graph for a run's snapshot.
#[test]
fn gets_snapshot_by_id() {
    let snapshot = draft_snapshot();
    let handler = GetWorkflowSnapshotHandler::new(Arc::new(SnapshotLookupRepository::found(
        snapshot.clone(),
    )));

    let response = handler
        .handle(ora_contracts::GetWorkflowSnapshotRequest {
            snapshot_id: "draft-1".to_string(),
        })
        .unwrap();

    assert_eq!(response.snapshot, map_snapshot(snapshot));
}

/// Verifies the snapshot-by-id handler reports not found for an unknown snapshot.
#[test]
fn gets_snapshot_by_id_reports_not_found() {
    let handler = GetWorkflowSnapshotHandler::new(Arc::new(SnapshotLookupRepository::missing()));

    let result = handler.handle(ora_contracts::GetWorkflowSnapshotRequest {
        snapshot_id: "snapshot-missing".to_string(),
    });

    assert_eq!(
        result.unwrap_err(),
        ApplicationError::WorkflowSnapshotNotFoundById {
            snapshot_id: "snapshot-missing".to_string(),
        }
    );
}
