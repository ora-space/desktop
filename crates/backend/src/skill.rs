use crate::clock::SystemClock;
use ora_application::{
    ApplicationError, CreateSkillHandler, DeleteSkillHandler, FilesystemSkillStorage,
    GetSkillHandler, ListSkillsHandler, UpdateSkillHandler, UuidSkillIdGenerator,
};
use ora_contracts::{
    CreateSkillRequest, CreateSkillResponse, DeleteSkillRequest, DeleteSkillResponse,
    GetSkillRequest, GetSkillResponse, ListSkillsRequest, ListSkillsResponse, UpdateSkillRequest,
    UpdateSkillResponse,
};
use ora_db::{RepositoryPool, SqliteSkillRepository};
use std::path::PathBuf;

/// Groups the concrete skill handlers shared by runtime adapters.
pub(crate) struct SkillApi {
    create: CreateSkillHandler<
        SqliteSkillRepository,
        FilesystemSkillStorage,
        UuidSkillIdGenerator,
        SystemClock,
    >,
    get: GetSkillHandler<SqliteSkillRepository>,
    list: ListSkillsHandler<SqliteSkillRepository>,
    update: UpdateSkillHandler<SqliteSkillRepository, FilesystemSkillStorage, SystemClock>,
    delete: DeleteSkillHandler<SqliteSkillRepository, FilesystemSkillStorage, SystemClock>,
}

impl SkillApi {
    /// Builds skill handlers from the shared repository pool and formal skills root.
    pub(crate) fn new(pool: RepositoryPool, skills_root: PathBuf, clock: SystemClock) -> Self {
        let repository = SqliteSkillRepository::new(pool);
        let storage = FilesystemSkillStorage::new(skills_root);

        Self {
            create: CreateSkillHandler::new(
                repository.clone(),
                storage.clone(),
                UuidSkillIdGenerator::new(),
                clock,
            ),
            get: GetSkillHandler::new(repository.clone()),
            list: ListSkillsHandler::new(repository.clone()),
            update: UpdateSkillHandler::new(repository.clone(), storage.clone(), clock),
            delete: DeleteSkillHandler::new(repository, storage, clock),
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
}
