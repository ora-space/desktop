use crate::clock::SystemClock;
use ora_application::{
    ApplicationError, CreateSkillHandler, DeleteSkillHandler, GetSkillHandler, ImportSkillHandler,
    ListSkillsHandler, LocalSkillPackageStore, UpdateSkillHandler, UploadedSkillFile,
    UuidSkillIdGenerator,
};
use ora_contracts::{
    CreateSkillRequest, CreateSkillResponse, DeleteSkillRequest, DeleteSkillResponse,
    GetSkillRequest, GetSkillResponse, ListSkillsRequest, ListSkillsResponse, UpdateSkillRequest,
    UpdateSkillResponse,
};
use ora_db::{RepositoryPool, SqliteSkillImportUnitOfWork, SqliteSkillRepository};

/// Groups the concrete skill handlers shared by runtime adapters.
pub(crate) struct SkillApi {
    create: CreateSkillHandler<SqliteSkillRepository, UuidSkillIdGenerator, SystemClock>,
    get: GetSkillHandler<SqliteSkillRepository>,
    list: ListSkillsHandler<SqliteSkillRepository>,
    update: UpdateSkillHandler<SqliteSkillRepository, SystemClock>,
    delete: DeleteSkillHandler<SqliteSkillRepository, LocalSkillPackageStore, SystemClock>,
    import: ImportSkillHandler<
        LocalSkillPackageStore,
        SqliteSkillImportUnitOfWork,
        UuidSkillIdGenerator,
        SystemClock,
    >,
}

impl SkillApi {
    /// Builds skill handlers from the shared repository pool and the skill package store.
    pub(crate) fn new(
        pool: RepositoryPool,
        clock: SystemClock,
        store: LocalSkillPackageStore,
    ) -> Self {
        let repository = SqliteSkillRepository::new(pool.clone());

        Self {
            create: CreateSkillHandler::new(repository.clone(), UuidSkillIdGenerator::new(), clock),
            get: GetSkillHandler::new(repository.clone()),
            list: ListSkillsHandler::new(repository.clone()),
            update: UpdateSkillHandler::new(repository.clone(), clock),
            delete: DeleteSkillHandler::new(repository, store.clone(), clock),
            import: ImportSkillHandler::new(
                store,
                SqliteSkillImportUnitOfWork::new(pool),
                UuidSkillIdGenerator::new(),
                clock,
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

    /// Executes one atomic skill folder import through the application handler.
    pub(crate) fn import(
        &self,
        files: Vec<UploadedSkillFile>,
    ) -> Result<CreateSkillResponse, ApplicationError> {
        self.import.handle(files)
    }
}
