use std::sync::Arc;

use ora_contracts::{
    ActivateWorkflowRequest, ActivateWorkflowResponse, CreateWorkflowRequest,
    CreateWorkflowResponse, DeleteSnapshotRequest, DeleteSnapshotResponse, DeleteWorkflowRequest,
    DeleteWorkflowResponse, GetDraftRequest, GetDraftResponse, GetVersionRequest,
    GetVersionResponse, GetWorkflowRequest, GetWorkflowResponse, ListVersionsRequest,
    ListVersionsResponse, ListWorkflowsRequest, ListWorkflowsResponse, PublishWorkflowRequest,
    PublishWorkflowResponse, RollbackWorkflowRequest, RollbackWorkflowResponse, UpdateDraftRequest,
    UpdateDraftResponse, UpdateWorkflowRequest, UpdateWorkflowResponse,
};
use ora_domain::{AuditFields, Workflow, WorkflowId, WorkflowSnapshot, WorkflowSnapshotId};

use crate::workflow::mapper::{
    map_created_workflow, map_snapshot, map_workflow, map_workflow_detail, map_workflow_summary,
    map_workflow_version,
};
use crate::workflow::ports::{
    ActivateVersionResult, DeleteSnapshotResult, DeleteWorkflowResult, PublishSnapshotResult,
    RollbackDraftResult, UpdateDraftResult, UpdateWorkflowResult, WorkflowIdGenerator,
    WorkflowRepository,
};
use crate::{ApplicationError, Clock};

const DRAFT_VERSION: &str = "draft";
const DEFAULT_GRAPH: &str = "{}";
const MAX_AUTOMATIC_VERSION_COLLISION_RETRIES: u16 = 100;

/// Handles creation of a new workflow with its initial draft snapshot.
pub struct CreateWorkflowHandler<Repository, IdGenerator, ClockSource> {
    repository: Repository,
    id_generator: IdGenerator,
    clock: ClockSource,
}

impl<Repository, IdGenerator, ClockSource>
    CreateWorkflowHandler<Repository, IdGenerator, ClockSource>
{
    pub fn new(repository: Repository, id_generator: IdGenerator, clock: ClockSource) -> Self {
        Self {
            repository,
            id_generator,
            clock,
        }
    }
}

impl<Repository, IdGenerator, ClockSource>
    CreateWorkflowHandler<Repository, IdGenerator, ClockSource>
where
    Repository: WorkflowRepository,
    IdGenerator: WorkflowIdGenerator,
    ClockSource: Clock,
{
    /// Creates a workflow and its initial draft, returning both.
    pub fn handle(
        &self,
        request: CreateWorkflowRequest,
    ) -> Result<CreateWorkflowResponse, ApplicationError> {
        let now = self.clock.now_timestamp_millis();
        let workflow_id = self.id_generator.generate_workflow_id();
        let snapshot_id = self.id_generator.generate_snapshot_id();
        let graph = request.graph.unwrap_or_else(|| DEFAULT_GRAPH.to_string());

        let workflow = Workflow::new(
            workflow_id.clone(),
            request.name,
            /*published_snapshot_id*/ None,
            AuditFields::new(now, now, /*is_deleted*/ false),
        )
        .map_err(ApplicationError::from_workflow_domain_error)?;

        let draft = WorkflowSnapshot::new(
            snapshot_id,
            workflow_id,
            DRAFT_VERSION,
            graph,
            now,
            Some(now),
            /*is_deleted*/ false,
        );

        let created = self
            .repository
            .create_workflow(workflow, draft)
            .map_err(ApplicationError::from_workflow_repository_error)?;

        Ok(map_created_workflow(created))
    }
}

/// Handles lookup of one workflow with its draft and published snapshot.
pub struct GetWorkflowHandler<Repository> {
    repository: Arc<Repository>,
}

impl<Repository> GetWorkflowHandler<Repository> {
    pub fn new(repository: Arc<Repository>) -> Self {
        Self { repository }
    }
}

impl<Repository> GetWorkflowHandler<Repository>
where
    Repository: WorkflowRepository + Send + Sync + 'static,
{
    /// Loads one workflow detail or reports a not-found error.
    pub fn handle(
        &self,
        request: GetWorkflowRequest,
    ) -> Result<GetWorkflowResponse, ApplicationError> {
        let workflow_id = WorkflowId::new(request.workflow_id);
        let detail = self
            .repository
            .get_workflow_detail(&workflow_id)
            .map_err(ApplicationError::from_workflow_repository_error)?
            .ok_or_else(|| ApplicationError::WorkflowNotFound {
                workflow_id: workflow_id.to_string(),
            })?;

        Ok(map_workflow_detail(detail))
    }
}

/// Handles listing of visible workflows.
pub struct ListWorkflowsHandler<Repository> {
    repository: Arc<Repository>,
}

impl<Repository> ListWorkflowsHandler<Repository> {
    pub fn new(repository: Arc<Repository>) -> Self {
        Self { repository }
    }
}

impl<Repository> ListWorkflowsHandler<Repository>
where
    Repository: WorkflowRepository + Send + Sync + 'static,
{
    /// Lists every visible workflow summary in storage order.
    pub fn handle(
        &self,
        _request: ListWorkflowsRequest,
    ) -> Result<ListWorkflowsResponse, ApplicationError> {
        let workflows = self
            .repository
            .list_workflows()
            .map_err(ApplicationError::from_workflow_repository_error)?;

        Ok(ListWorkflowsResponse {
            workflows: workflows.into_iter().map(map_workflow_summary).collect(),
        })
    }
}

/// Handles replacement of a workflow's editable fields (currently only name).
pub struct UpdateWorkflowHandler<Repository, ClockSource> {
    repository: Arc<Repository>,
    clock: ClockSource,
}

impl<Repository, ClockSource> UpdateWorkflowHandler<Repository, ClockSource> {
    pub fn new(repository: Arc<Repository>, clock: ClockSource) -> Self {
        Self { repository, clock }
    }
}

impl<Repository, ClockSource> UpdateWorkflowHandler<Repository, ClockSource>
where
    Repository: WorkflowRepository + Send + Sync + 'static,
    ClockSource: Clock,
{
    /// Replaces the workflow name while preserving its identifier and creation timestamp.
    pub fn handle(
        &self,
        request: UpdateWorkflowRequest,
    ) -> Result<UpdateWorkflowResponse, ApplicationError> {
        let workflow_id = WorkflowId::new(request.workflow_id);
        let existing = self
            .repository
            .find_workflow(&workflow_id)
            .map_err(ApplicationError::from_workflow_repository_error)?
            .ok_or_else(|| ApplicationError::WorkflowNotFound {
                workflow_id: workflow_id.to_string(),
            })?;

        let workflow = Workflow::new(
            workflow_id.clone(),
            request.name,
            existing.published_snapshot_id,
            AuditFields::new(
                existing.audit_fields.created_at,
                self.clock.now_timestamp_millis(),
                /*is_deleted*/ false,
            ),
        )
        .map_err(ApplicationError::from_workflow_domain_error)?;

        let updated = self
            .repository
            .update_workflow(
                &workflow_id,
                workflow.name,
                workflow.audit_fields.updated_at,
            )
            .map_err(ApplicationError::from_workflow_repository_error)?;

        let UpdateWorkflowResult::Updated(updated) = updated else {
            return Err(ApplicationError::WorkflowNotFound {
                workflow_id: workflow_id.to_string(),
            });
        };

        Ok(UpdateWorkflowResponse {
            workflow: map_workflow(updated),
        })
    }
}

/// Handles soft-deletion of a workflow with cascade to all its snapshots.
pub struct DeleteWorkflowHandler<Repository, ClockSource> {
    repository: Arc<Repository>,
    clock: ClockSource,
}

impl<Repository, ClockSource> DeleteWorkflowHandler<Repository, ClockSource> {
    pub fn new(repository: Arc<Repository>, clock: ClockSource) -> Self {
        Self { repository, clock }
    }
}

impl<Repository, ClockSource> DeleteWorkflowHandler<Repository, ClockSource>
where
    Repository: WorkflowRepository + Send + Sync + 'static,
    ClockSource: Clock,
{
    /// Soft-deletes one visible workflow and cascades to all its snapshots.
    pub fn handle(
        &self,
        request: DeleteWorkflowRequest,
    ) -> Result<DeleteWorkflowResponse, ApplicationError> {
        let workflow_id = WorkflowId::new(request.workflow_id);
        let deleted = self
            .repository
            .soft_delete_workflow(&workflow_id, self.clock.now_timestamp_millis())
            .map_err(ApplicationError::from_workflow_repository_error)?;

        match deleted {
            DeleteWorkflowResult::Deleted => {}
            DeleteWorkflowResult::NotFound => {
                return Err(ApplicationError::WorkflowNotFound {
                    workflow_id: workflow_id.to_string(),
                });
            }
            DeleteWorkflowResult::ActiveRuns => {
                return Err(ApplicationError::WorkflowActiveRuns);
            }
        }

        Ok(DeleteWorkflowResponse {
            workflow_id: workflow_id.to_string(),
        })
    }
}

/// Handles retrieval of the draft snapshot for a workflow.
pub struct GetDraftHandler<Repository> {
    repository: Arc<Repository>,
}

impl<Repository> GetDraftHandler<Repository> {
    pub fn new(repository: Arc<Repository>) -> Self {
        Self { repository }
    }
}

impl<Repository> GetDraftHandler<Repository>
where
    Repository: WorkflowRepository + Send + Sync + 'static,
{
    /// Loads the draft snapshot including its full graph.
    pub fn handle(&self, request: GetDraftRequest) -> Result<GetDraftResponse, ApplicationError> {
        let workflow_id = WorkflowId::new(request.workflow_id);
        let snapshot = self
            .repository
            .find_snapshot_by_version(&workflow_id, DRAFT_VERSION)
            .map_err(ApplicationError::from_workflow_repository_error)?
            .ok_or_else(|| ApplicationError::WorkflowNotFound {
                workflow_id: workflow_id.to_string(),
            })?;

        Ok(GetDraftResponse {
            snapshot: map_snapshot(snapshot),
        })
    }
}

/// Handles in-place update of the draft snapshot's graph.
pub struct UpdateDraftHandler<Repository, ClockSource> {
    repository: Arc<Repository>,
    clock: ClockSource,
}

impl<Repository, ClockSource> UpdateDraftHandler<Repository, ClockSource> {
    pub fn new(repository: Arc<Repository>, clock: ClockSource) -> Self {
        Self { repository, clock }
    }
}

impl<Repository, ClockSource> UpdateDraftHandler<Repository, ClockSource>
where
    Repository: WorkflowRepository + Send + Sync + 'static,
    ClockSource: Clock,
{
    /// Updates the draft's graph in-place without creating a new snapshot.
    pub fn handle(
        &self,
        request: UpdateDraftRequest,
    ) -> Result<UpdateDraftResponse, ApplicationError> {
        let workflow_id = WorkflowId::new(request.workflow_id);
        let updated = self
            .repository
            .update_draft(
                &workflow_id,
                request.graph,
                self.clock.now_timestamp_millis(),
            )
            .map_err(ApplicationError::from_workflow_repository_error)?;

        let updated = match updated {
            UpdateDraftResult::Updated(snapshot) => snapshot,
            UpdateDraftResult::WorkflowNotFound => {
                return Err(ApplicationError::WorkflowNotFound {
                    workflow_id: workflow_id.to_string(),
                });
            }
            UpdateDraftResult::DraftNotFound => {
                return Err(ApplicationError::WorkflowSnapshotNotFound {
                    workflow_id: workflow_id.to_string(),
                    version: DRAFT_VERSION.to_string(),
                });
            }
        };

        Ok(UpdateDraftResponse {
            snapshot: map_snapshot(updated),
        })
    }
}

/// Handles publishing the draft as an immutable versioned snapshot.
pub struct PublishWorkflowHandler<Repository, IdGenerator, ClockSource> {
    repository: Arc<Repository>,
    id_generator: IdGenerator,
    clock: ClockSource,
}

impl<Repository, IdGenerator, ClockSource>
    PublishWorkflowHandler<Repository, IdGenerator, ClockSource>
{
    pub fn new(repository: Arc<Repository>, id_generator: IdGenerator, clock: ClockSource) -> Self {
        Self {
            repository,
            id_generator,
            clock,
        }
    }
}

impl<Repository, IdGenerator, ClockSource>
    PublishWorkflowHandler<Repository, IdGenerator, ClockSource>
where
    Repository: WorkflowRepository + Send + Sync + 'static,
    IdGenerator: WorkflowIdGenerator,
    ClockSource: Clock,
{
    /// Publishes the draft, creating an immutable snapshot and activating it.
    pub fn handle(
        &self,
        request: PublishWorkflowRequest,
    ) -> Result<PublishWorkflowResponse, ApplicationError> {
        let workflow_id = WorkflowId::new(request.workflow_id);

        let now = self.clock.now_timestamp_millis();
        let snapshot_id = self.id_generator.generate_snapshot_id();

        // User-defined versions must remain safe to address through a single URL path segment.
        let (mut version, is_automatic_version) = match request.version {
            Some(ref v) if v == DRAFT_VERSION => {
                return Err(ApplicationError::WorkflowVersionReserved);
            }
            Some(v) => {
                if v.trim().is_empty()
                    || v.len() > 128
                    || matches!(v.as_str(), "." | "..")
                    || v.chars()
                        .any(|character| character.is_control() || matches!(character, '/' | '\\'))
                {
                    return Err(ApplicationError::WorkflowVersionInvalid);
                }
                (v, false)
            }
            None => (format!("v{now}"), true),
        };

        for collision_retry in 0..=MAX_AUTOMATIC_VERSION_COLLISION_RETRIES {
            let created = self
                .repository
                .publish_snapshot(&workflow_id, snapshot_id.clone(), version.clone(), now)
                .map_err(ApplicationError::from_workflow_repository_error)?;

            match created {
                PublishSnapshotResult::Published(snapshot) => {
                    return Ok(PublishWorkflowResponse {
                        snapshot: map_snapshot(snapshot),
                    });
                }
                PublishSnapshotResult::WorkflowNotFound => {
                    return Err(ApplicationError::WorkflowNotFound {
                        workflow_id: workflow_id.to_string(),
                    });
                }
                PublishSnapshotResult::DraftNotFound => {
                    return Err(ApplicationError::WorkflowSnapshotNotFound {
                        workflow_id: workflow_id.to_string(),
                        version: DRAFT_VERSION.to_string(),
                    });
                }
                PublishSnapshotResult::VersionAlreadyExists
                    if is_automatic_version
                        && collision_retry < MAX_AUTOMATIC_VERSION_COLLISION_RETRIES =>
                {
                    // Keep the version tied to the injected clock while making same-millisecond
                    // automatic publishes distinct without changing user-provided version names.
                    version = format!("v{now}-{}", collision_retry + 1);
                }
                PublishSnapshotResult::VersionAlreadyExists => {
                    return Err(ApplicationError::WorkflowVersionAlreadyExists {
                        workflow_id: workflow_id.to_string(),
                        version,
                    });
                }
            }
        }

        unreachable!("automatic version collision retries always return from the loop")
    }
}

/// Handles rolling back the draft to a historical snapshot's graph.
pub struct RollbackWorkflowHandler<Repository, ClockSource> {
    repository: Arc<Repository>,
    clock: ClockSource,
}

impl<Repository, ClockSource> RollbackWorkflowHandler<Repository, ClockSource> {
    pub fn new(repository: Arc<Repository>, clock: ClockSource) -> Self {
        Self { repository, clock }
    }
}

impl<Repository, ClockSource> RollbackWorkflowHandler<Repository, ClockSource>
where
    Repository: WorkflowRepository + Send + Sync + 'static,
    ClockSource: Clock,
{
    /// Copies a historical snapshot's graph into the draft without changing the published pointer.
    pub fn handle(
        &self,
        request: RollbackWorkflowRequest,
    ) -> Result<RollbackWorkflowResponse, ApplicationError> {
        let workflow_id = WorkflowId::new(request.workflow_id);
        let snapshot_id = WorkflowSnapshotId::new(request.snapshot_id);

        let updated = self
            .repository
            .rollback_draft(
                &workflow_id,
                &snapshot_id,
                self.clock.now_timestamp_millis(),
            )
            .map_err(ApplicationError::from_workflow_repository_error)?;

        let updated = match updated {
            RollbackDraftResult::DraftUpdated(snapshot) => snapshot,
            RollbackDraftResult::WorkflowNotFound => {
                return Err(ApplicationError::WorkflowNotFound {
                    workflow_id: workflow_id.to_string(),
                });
            }
            RollbackDraftResult::SnapshotNotFound => {
                return Err(ApplicationError::WorkflowSnapshotNotFound {
                    workflow_id: workflow_id.to_string(),
                    version: snapshot_id.to_string(),
                });
            }
            RollbackDraftResult::DraftSnapshot => {
                return Err(ApplicationError::WorkflowCannotRollbackToDraft);
            }
            RollbackDraftResult::DraftNotFound => {
                return Err(ApplicationError::WorkflowSnapshotNotFound {
                    workflow_id: workflow_id.to_string(),
                    version: DRAFT_VERSION.to_string(),
                });
            }
        };

        Ok(RollbackWorkflowResponse {
            snapshot: map_snapshot(updated),
        })
    }
}

/// Handles switching the published version pointer and syncing the draft.
pub struct ActivateWorkflowHandler<Repository, ClockSource> {
    repository: Arc<Repository>,
    clock: ClockSource,
}

impl<Repository, ClockSource> ActivateWorkflowHandler<Repository, ClockSource> {
    pub fn new(repository: Arc<Repository>, clock: ClockSource) -> Self {
        Self { repository, clock }
    }
}

impl<Repository, ClockSource> ActivateWorkflowHandler<Repository, ClockSource>
where
    Repository: WorkflowRepository + Send + Sync + 'static,
    ClockSource: Clock,
{
    /// Activates a published snapshot and syncs its graph into the draft.
    pub fn handle(
        &self,
        request: ActivateWorkflowRequest,
    ) -> Result<ActivateWorkflowResponse, ApplicationError> {
        let workflow_id = WorkflowId::new(request.workflow_id);
        let snapshot_id = WorkflowSnapshotId::new(request.snapshot_id);

        let updated = self
            .repository
            .activate_version(
                &workflow_id,
                &snapshot_id,
                self.clock.now_timestamp_millis(),
            )
            .map_err(ApplicationError::from_workflow_repository_error)?;

        let updated = match updated {
            ActivateVersionResult::Activated(snapshot) => snapshot,
            ActivateVersionResult::WorkflowNotFound => {
                return Err(ApplicationError::WorkflowNotFound {
                    workflow_id: workflow_id.to_string(),
                });
            }
            ActivateVersionResult::SnapshotNotFound => {
                return Err(ApplicationError::WorkflowSnapshotNotFound {
                    workflow_id: workflow_id.to_string(),
                    version: snapshot_id.to_string(),
                });
            }
            ActivateVersionResult::DraftSnapshot => {
                return Err(ApplicationError::WorkflowCannotActivateDraft);
            }
            ActivateVersionResult::DraftNotFound => {
                return Err(ApplicationError::WorkflowSnapshotNotFound {
                    workflow_id: workflow_id.to_string(),
                    version: DRAFT_VERSION.to_string(),
                });
            }
        };

        Ok(ActivateWorkflowResponse {
            snapshot: map_snapshot(updated),
        })
    }
}

/// Handles listing published (non-draft, non-deleted) version summaries.
pub struct ListVersionsHandler<Repository> {
    repository: Arc<Repository>,
}

impl<Repository> ListVersionsHandler<Repository> {
    pub fn new(repository: Arc<Repository>) -> Self {
        Self { repository }
    }
}

impl<Repository> ListVersionsHandler<Repository>
where
    Repository: WorkflowRepository + Send + Sync + 'static,
{
    /// Lists published version summaries for a workflow.
    pub fn handle(
        &self,
        request: ListVersionsRequest,
    ) -> Result<ListVersionsResponse, ApplicationError> {
        let workflow_id = WorkflowId::new(request.workflow_id);
        let versions = self
            .repository
            .list_versions(&workflow_id)
            .map_err(ApplicationError::from_workflow_repository_error)?;

        Ok(ListVersionsResponse {
            versions: versions.into_iter().map(map_workflow_version).collect(),
        })
    }
}

/// Handles retrieval of a specific snapshot by version string.
pub struct GetVersionHandler<Repository> {
    repository: Arc<Repository>,
}

impl<Repository> GetVersionHandler<Repository> {
    pub fn new(repository: Arc<Repository>) -> Self {
        Self { repository }
    }
}

impl<Repository> GetVersionHandler<Repository>
where
    Repository: WorkflowRepository + Send + Sync + 'static,
{
    /// Loads one snapshot (draft or published) by its version string.
    pub fn handle(
        &self,
        request: GetVersionRequest,
    ) -> Result<GetVersionResponse, ApplicationError> {
        let workflow_id = WorkflowId::new(request.workflow_id);
        let snapshot = self
            .repository
            .find_snapshot_by_version(&workflow_id, &request.version)
            .map_err(ApplicationError::from_workflow_repository_error)?
            .ok_or_else(|| ApplicationError::WorkflowSnapshotNotFound {
                workflow_id: workflow_id.to_string(),
                version: request.version.clone(),
            })?;

        Ok(GetVersionResponse {
            snapshot: map_snapshot(snapshot),
        })
    }
}

/// Handles soft-deletion of a published snapshot, subject to constraints.
pub struct DeleteSnapshotHandler<Repository, ClockSource> {
    repository: Arc<Repository>,
    clock: ClockSource,
}

impl<Repository, ClockSource> DeleteSnapshotHandler<Repository, ClockSource> {
    pub fn new(repository: Arc<Repository>, clock: ClockSource) -> Self {
        Self { repository, clock }
    }
}

impl<Repository, ClockSource> DeleteSnapshotHandler<Repository, ClockSource>
where
    Repository: WorkflowRepository + Send + Sync + 'static,
    ClockSource: Clock,
{
    /// Soft-deletes a published snapshot that is not the draft and not the active version.
    pub fn handle(
        &self,
        request: DeleteSnapshotRequest,
    ) -> Result<DeleteSnapshotResponse, ApplicationError> {
        let workflow_id = WorkflowId::new(request.workflow_id);
        self.repository
            .find_workflow(&workflow_id)
            .map_err(ApplicationError::from_workflow_repository_error)?
            .ok_or_else(|| ApplicationError::WorkflowNotFound {
                workflow_id: workflow_id.to_string(),
            })?;
        let snapshot = self
            .repository
            .find_snapshot_by_version(&workflow_id, &request.version)
            .map_err(ApplicationError::from_workflow_repository_error)?
            .ok_or_else(|| ApplicationError::WorkflowSnapshotNotFound {
                workflow_id: workflow_id.to_string(),
                version: request.version.clone(),
            })?;

        let deletion = self
            .repository
            .soft_delete_snapshot(
                &workflow_id,
                &snapshot.id,
                self.clock.now_timestamp_millis(),
            )
            .map_err(ApplicationError::from_workflow_repository_error)?;
        match deletion {
            DeleteSnapshotResult::Deleted(snapshot) => Ok(DeleteSnapshotResponse {
                snapshot_id: snapshot.id.to_string(),
                version: snapshot.version,
            }),
            DeleteSnapshotResult::WorkflowNotFound => Err(ApplicationError::WorkflowNotFound {
                workflow_id: workflow_id.to_string(),
            }),
            DeleteSnapshotResult::SnapshotNotFound => {
                Err(ApplicationError::WorkflowSnapshotNotFound {
                    workflow_id: workflow_id.to_string(),
                    version: request.version,
                })
            }
            DeleteSnapshotResult::DraftSnapshot => Err(ApplicationError::WorkflowCannotDeleteDraft),
            DeleteSnapshotResult::ActiveSnapshot => {
                Err(ApplicationError::WorkflowCannotDeleteActiveVersion)
            }
            DeleteSnapshotResult::SnapshotInUse => Err(ApplicationError::WorkflowSnapshotInUse),
        }
    }
}
