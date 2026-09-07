use crate::BackendError;
use crate::clock::SystemClock;
use ora_application::{
    ActivateWorkflowHandler, CreateWorkflowHandler, DeleteSnapshotHandler, DeleteWorkflowHandler,
    GetDraftHandler, GetVersionHandler, GetWorkflowHandler, GetWorkflowSnapshotHandler,
    ListVersionsHandler, ListWorkflowsHandler, PublishWorkflowHandler, RollbackWorkflowHandler,
    UpdateDraftHandler, UpdateWorkflowHandler, UuidWorkflowIdGenerator,
};
use ora_contracts::{
    ActivateWorkflowRequest, ActivateWorkflowResponse, CreateWorkflowRequest,
    CreateWorkflowResponse, DeleteSnapshotRequest, DeleteSnapshotResponse, DeleteWorkflowRequest,
    DeleteWorkflowResponse, GetDraftRequest, GetDraftResponse, GetVersionRequest,
    GetVersionResponse, GetWorkflowRequest, GetWorkflowResponse, GetWorkflowSnapshotRequest,
    GetWorkflowSnapshotResponse, ListVersionsRequest, ListVersionsResponse, ListWorkflowsRequest,
    ListWorkflowsResponse, PublishWorkflowRequest, PublishWorkflowResponse,
    RollbackWorkflowRequest, RollbackWorkflowResponse, UpdateDraftRequest, UpdateDraftResponse,
    UpdateWorkflowRequest, UpdateWorkflowResponse,
};
use ora_db::{RepositoryPool, SqliteWorkflowRepository};
use std::sync::Arc;

/// Groups the concrete workflow handlers shared by runtime adapters.
pub struct WorkflowApi {
    create: CreateWorkflowHandler<SqliteWorkflowRepository, UuidWorkflowIdGenerator, SystemClock>,
    get: GetWorkflowHandler<SqliteWorkflowRepository>,
    list: ListWorkflowsHandler<SqliteWorkflowRepository>,
    update: UpdateWorkflowHandler<SqliteWorkflowRepository, SystemClock>,
    delete: DeleteWorkflowHandler<SqliteWorkflowRepository, SystemClock>,
    get_draft: GetDraftHandler<SqliteWorkflowRepository>,
    update_draft: UpdateDraftHandler<SqliteWorkflowRepository, SystemClock>,
    publish: PublishWorkflowHandler<SqliteWorkflowRepository, UuidWorkflowIdGenerator, SystemClock>,
    rollback: RollbackWorkflowHandler<SqliteWorkflowRepository, SystemClock>,
    activate: ActivateWorkflowHandler<SqliteWorkflowRepository, SystemClock>,
    list_versions: ListVersionsHandler<SqliteWorkflowRepository>,
    get_version: GetVersionHandler<SqliteWorkflowRepository>,
    get_snapshot: GetWorkflowSnapshotHandler<SqliteWorkflowRepository>,
    delete_snapshot: DeleteSnapshotHandler<SqliteWorkflowRepository, SystemClock>,
}

impl WorkflowApi {
    /// Builds workflow handlers from the shared repository pool.
    pub(crate) fn new(pool: RepositoryPool, clock: SystemClock) -> Self {
        let repository = Arc::new(SqliteWorkflowRepository::new(pool));
        let id_generator = UuidWorkflowIdGenerator::new();

        Self {
            create: CreateWorkflowHandler::new((*repository).clone(), id_generator.clone(), clock),
            get: GetWorkflowHandler::new(repository.clone()),
            list: ListWorkflowsHandler::new(repository.clone()),
            update: UpdateWorkflowHandler::new(repository.clone(), clock),
            delete: DeleteWorkflowHandler::new(repository.clone(), clock),
            get_draft: GetDraftHandler::new(repository.clone()),
            update_draft: UpdateDraftHandler::new(repository.clone(), clock),
            publish: PublishWorkflowHandler::new(repository.clone(), id_generator, clock),
            rollback: RollbackWorkflowHandler::new(repository.clone(), clock),
            activate: ActivateWorkflowHandler::new(repository.clone(), clock),
            list_versions: ListVersionsHandler::new(repository.clone()),
            get_version: GetVersionHandler::new(repository.clone()),
            get_snapshot: GetWorkflowSnapshotHandler::new(repository.clone()),
            delete_snapshot: DeleteSnapshotHandler::new(repository, clock),
        }
    }

    /// Creates a definition and its draft without starting a workflow run.
    pub fn create(
        &self,
        request: CreateWorkflowRequest,
    ) -> Result<CreateWorkflowResponse, BackendError> {
        self.create.handle(request).map_err(BackendError::from)
    }

    /// Loads a visible definition while preserving the application not-found semantics.
    pub fn get(&self, request: GetWorkflowRequest) -> Result<GetWorkflowResponse, BackendError> {
        self.get.handle(request).map_err(BackendError::from)
    }

    /// Lists visible definitions without loading execution state.
    pub fn list(
        &self,
        request: ListWorkflowsRequest,
    ) -> Result<ListWorkflowsResponse, BackendError> {
        self.list.handle(request).map_err(BackendError::from)
    }

    /// Updates definition metadata while leaving snapshot identity under application control.
    pub fn update(
        &self,
        request: UpdateWorkflowRequest,
    ) -> Result<UpdateWorkflowResponse, BackendError> {
        self.update.handle(request).map_err(BackendError::from)
    }

    /// Deletes a definition with its application-owned reference checks.
    pub fn delete(
        &self,
        request: DeleteWorkflowRequest,
    ) -> Result<DeleteWorkflowResponse, BackendError> {
        self.delete.handle(request).map_err(BackendError::from)
    }

    /// Loads the mutable draft separately from immutable published versions.
    pub fn get_draft(&self, request: GetDraftRequest) -> Result<GetDraftResponse, BackendError> {
        self.get_draft.handle(request).map_err(BackendError::from)
    }

    /// Replaces draft graph content under the existing graph-validation rules.
    pub fn update_draft(
        &self,
        request: UpdateDraftRequest,
    ) -> Result<UpdateDraftResponse, BackendError> {
        self.update_draft
            .handle(request)
            .map_err(BackendError::from)
    }

    /// Validates the draft and creates the next immutable published snapshot.
    pub fn publish(
        &self,
        request: PublishWorkflowRequest,
    ) -> Result<PublishWorkflowResponse, BackendError> {
        self.publish.handle(request).map_err(BackendError::from)
    }

    /// Restores a selected version into the editable draft.
    pub fn rollback(
        &self,
        request: RollbackWorkflowRequest,
    ) -> Result<RollbackWorkflowResponse, BackendError> {
        self.rollback.handle(request).map_err(BackendError::from)
    }

    /// Selects the published version used by future runs without rewriting existing runs.
    pub fn activate(
        &self,
        request: ActivateWorkflowRequest,
    ) -> Result<ActivateWorkflowResponse, BackendError> {
        self.activate.handle(request).map_err(BackendError::from)
    }

    /// Lists immutable published snapshots belonging to one definition.
    pub fn list_versions(
        &self,
        request: ListVersionsRequest,
    ) -> Result<ListVersionsResponse, BackendError> {
        self.list_versions
            .handle(request)
            .map_err(BackendError::from)
    }

    /// Resolves one version within its owning definition.
    pub fn get_version(
        &self,
        request: GetVersionRequest,
    ) -> Result<GetVersionResponse, BackendError> {
        self.get_version.handle(request).map_err(BackendError::from)
    }

    /// Deletes a snapshot only when application reference checks allow it.
    pub fn delete_snapshot(
        &self,
        request: DeleteSnapshotRequest,
    ) -> Result<DeleteSnapshotResponse, BackendError> {
        self.delete_snapshot
            .handle(request)
            .map_err(BackendError::from)
    }

    /// Loads a snapshot through the same visibility rules as other definition reads.
    pub fn get_snapshot(
        &self,
        request: GetWorkflowSnapshotRequest,
    ) -> Result<GetWorkflowSnapshotResponse, BackendError> {
        self.get_snapshot
            .handle(request)
            .map_err(BackendError::from)
    }
}

#[cfg(test)]
mod tests;
