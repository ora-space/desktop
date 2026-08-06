use crate::clock::SystemClock;
use ora_application::{
    ApplicationError, CreateSkillHandler, DeleteSkillHandler, FilesystemSkillStorage,
    GetSkillHandler, ListSkillsHandler, NoopSkillImportProgressPublisher, SkillImportConfig,
    SkillImportService, UpdateSkillHandler, UuidSkillIdGenerator, UuidSkillImportIdGenerator,
};
use ora_contracts::{
    CancelSkillImportRequest, CancelSkillImportResponse, CommitSkillImportRequest,
    CommitSkillImportResponse, CreateSkillRequest, CreateSkillResponse, DeleteSkillRequest,
    DeleteSkillResponse, GetSkillImportSessionRequest, GetSkillImportSessionResponse,
    GetSkillRequest, GetSkillResponse, ListSkillsRequest, ListSkillsResponse,
    PrepareSkillImportRequest, PrepareSkillImportResponse, UpdateSkillRequest, UpdateSkillResponse,
};
use ora_db::{RepositoryPool, SqliteSkillRepository};
use std::path::PathBuf;

/// Groups the concrete skill handlers and import service shared by runtime adapters.
pub(crate) struct SkillApi {
    create: CreateSkillHandler<
        SqliteSkillRepository,
        FilesystemSkillStorage,
        UuidSkillIdGenerator,
        SystemClock,
    >,
    get: GetSkillHandler<SqliteSkillRepository, FilesystemSkillStorage>,
    list: ListSkillsHandler<SqliteSkillRepository>,
    update: UpdateSkillHandler<SqliteSkillRepository, FilesystemSkillStorage, SystemClock>,
    delete: DeleteSkillHandler<SqliteSkillRepository, FilesystemSkillStorage, SystemClock>,
    import: SkillImportService<
        SqliteSkillRepository,
        FilesystemSkillStorage,
        UuidSkillImportIdGenerator,
        SystemClock,
        NoopSkillImportProgressPublisher,
    >,
}

impl SkillApi {
    /// Builds skill handlers from the shared repository pool and formal skills root.
    pub(crate) fn new(pool: RepositoryPool, skills_root: PathBuf, clock: SystemClock) -> Self {
        let repository = SqliteSkillRepository::new(pool);
        let storage = FilesystemSkillStorage::new(skills_root.clone());

        Self {
            create: CreateSkillHandler::new(
                repository.clone(),
                storage.clone(),
                UuidSkillIdGenerator::new(),
                clock,
            ),
            get: GetSkillHandler::new(repository.clone(), storage.clone()),
            list: ListSkillsHandler::new(repository.clone()),
            update: UpdateSkillHandler::new(repository.clone(), storage.clone(), clock),
            delete: DeleteSkillHandler::new(repository.clone(), storage, clock),
            import: SkillImportService::new(
                repository,
                FilesystemSkillStorage::new(skills_root),
                UuidSkillImportIdGenerator,
                clock,
                NoopSkillImportProgressPublisher,
                SkillImportConfig::default(),
            ),
        }
    }

    /// Executes skill creation through the application handler.
    pub(crate) fn create(
        &self,
        request: CreateSkillRequest,
    ) -> Result<CreateSkillResponse, ApplicationError> {
        self.create.handle(request)
    }

    /// Executes one skill lookup through the application handler.
    pub(crate) fn get(
        &self,
        request: GetSkillRequest,
    ) -> Result<GetSkillResponse, ApplicationError> {
        self.get.handle(request)
    }

    /// Executes skill listing through the application handler.
    pub(crate) fn list(
        &self,
        request: ListSkillsRequest,
    ) -> Result<ListSkillsResponse, ApplicationError> {
        self.list.handle(request)
    }

    /// Executes skill replacement through the application handler.
    pub(crate) fn update(
        &self,
        request: UpdateSkillRequest,
    ) -> Result<UpdateSkillResponse, ApplicationError> {
        self.update.handle(request)
    }

    /// Executes skill deletion through the application handler.
    pub(crate) fn delete(
        &self,
        request: DeleteSkillRequest,
    ) -> Result<DeleteSkillResponse, ApplicationError> {
        self.delete.handle(request)
    }

    /// Prepares one import source into a previewed session.
    pub(crate) fn prepare_import(
        &self,
        request: PrepareSkillImportRequest,
    ) -> Result<PrepareSkillImportResponse, ApplicationError> {
        self.import.prepare(request)
    }

    /// Returns one import session projection.
    pub(crate) fn get_import(
        &self,
        request: GetSkillImportSessionRequest,
    ) -> Result<GetSkillImportSessionResponse, ApplicationError> {
        self.import.get_session(request)
    }

    /// Accepts and freezes one import commit.
    pub(crate) fn commit_import(
        &self,
        request: CommitSkillImportRequest,
    ) -> Result<CommitSkillImportResponse, ApplicationError> {
        self.import.commit(request)
    }

    /// Cancels one prepared import session.
    pub(crate) fn cancel_import(
        &self,
        request: CancelSkillImportRequest,
    ) -> Result<CancelSkillImportResponse, ApplicationError> {
        self.import.cancel(request)
    }
}
