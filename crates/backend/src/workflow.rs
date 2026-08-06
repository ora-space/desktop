use crate::clock::SystemClock;
use ora_application::{
    ActivateWorkflowHandler, ApplicationError, CreateWorkflowHandler, DeleteSnapshotHandler,
    DeleteWorkflowHandler, GetDraftHandler, GetVersionHandler, GetWorkflowHandler,
    GetWorkflowSnapshotHandler, ListVersionsHandler, ListWorkflowsHandler, PublishWorkflowHandler,
    RollbackWorkflowHandler, UpdateDraftHandler, UpdateWorkflowHandler, UuidWorkflowIdGenerator,
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
pub(crate) struct WorkflowApi {
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

    pub(crate) fn create(
        &self,
        request: CreateWorkflowRequest,
    ) -> Result<CreateWorkflowResponse, ApplicationError> {
        self.create.handle(request)
    }

    pub(crate) fn get(
        &self,
        request: GetWorkflowRequest,
    ) -> Result<GetWorkflowResponse, ApplicationError> {
        self.get.handle(request)
    }

    pub(crate) fn list(
        &self,
        request: ListWorkflowsRequest,
    ) -> Result<ListWorkflowsResponse, ApplicationError> {
        self.list.handle(request)
    }

    pub(crate) fn update(
        &self,
        request: UpdateWorkflowRequest,
    ) -> Result<UpdateWorkflowResponse, ApplicationError> {
        self.update.handle(request)
    }

    pub(crate) fn delete(
        &self,
        request: DeleteWorkflowRequest,
    ) -> Result<DeleteWorkflowResponse, ApplicationError> {
        self.delete.handle(request)
    }

    pub(crate) fn get_draft(
        &self,
        request: GetDraftRequest,
    ) -> Result<GetDraftResponse, ApplicationError> {
        self.get_draft.handle(request)
    }

    pub(crate) fn update_draft(
        &self,
        request: UpdateDraftRequest,
    ) -> Result<UpdateDraftResponse, ApplicationError> {
        self.update_draft.handle(request)
    }

    pub(crate) fn publish(
        &self,
        request: PublishWorkflowRequest,
    ) -> Result<PublishWorkflowResponse, ApplicationError> {
        self.publish.handle(request)
    }

    pub(crate) fn rollback(
        &self,
        request: RollbackWorkflowRequest,
    ) -> Result<RollbackWorkflowResponse, ApplicationError> {
        self.rollback.handle(request)
    }

    pub(crate) fn activate(
        &self,
        request: ActivateWorkflowRequest,
    ) -> Result<ActivateWorkflowResponse, ApplicationError> {
        self.activate.handle(request)
    }

    pub(crate) fn list_versions(
        &self,
        request: ListVersionsRequest,
    ) -> Result<ListVersionsResponse, ApplicationError> {
        self.list_versions.handle(request)
    }

    pub(crate) fn get_version(
        &self,
        request: GetVersionRequest,
    ) -> Result<GetVersionResponse, ApplicationError> {
        self.get_version.handle(request)
    }

    pub(crate) fn delete_snapshot(
        &self,
        request: DeleteSnapshotRequest,
    ) -> Result<DeleteSnapshotResponse, ApplicationError> {
        self.delete_snapshot.handle(request)
    }

    pub(crate) fn get_snapshot(
        &self,
        request: GetWorkflowSnapshotRequest,
    ) -> Result<GetWorkflowSnapshotResponse, ApplicationError> {
        self.get_snapshot.handle(request)
    }
}
