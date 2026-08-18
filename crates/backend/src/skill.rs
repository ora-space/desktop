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
    list: ListSkillsHandler<SqliteSkillRepository, FilesystemSkillStorage>,
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
            list: ListSkillsHandler::new(repository.clone(), storage.clone()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use ora_application::{
        AgentDefinitionRepository, CreateHandle, DeleteHandle, SkillRepository, SkillStorage,
        SkillStorageError, SwapHandle, TransactionJournal,
    };
    use ora_db::{
        DatabaseBootstrapper, DatabaseLocation, SqliteAgentDefinitionRepository,
        default_migration_catalog,
    };
    use ora_domain::{AgentDefinition, AgentDefinitionId, AuditFields, Namespace, SkillId};
    use ora_logging::with_trace_logging;
    use ora_utils::path::StrictRelativePath;
    use pretty_assertions::assert_eq;
    use std::path::Path;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use tempfile::TempDir;

    /// Delegates every storage operation but parks `commit_create` on a barrier.
    ///
    /// The park lets a test hold the package promote open for as long as it wants, which is the
    /// slow-storage case the promote ordering has to tolerate.
    struct PausingSkillStorage {
        inner: FilesystemSkillStorage,
        promote: Arc<Barrier>,
    }

    impl SkillStorage for PausingSkillStorage {
        fn commit_create(
            &self,
            name: &str,
            skill_id: &SkillId,
            staging: &Path,
        ) -> Result<CreateHandle, SkillStorageError> {
            let handle = self.inner.commit_create(name, skill_id, staging)?;
            // Enter the promote window, then hold it until the observing thread releases it.
            self.promote.wait();
            self.promote.wait();
            Ok(handle)
        }

        fn create_staging(&self) -> Result<std::path::PathBuf, SkillStorageError> {
            self.inner.create_staging()
        }

        fn stage_existing(&self, name: &str, staging: &Path) -> Result<(), SkillStorageError> {
            self.inner.stage_existing(name, staging)
        }

        fn write_file(
            &self,
            staging: &Path,
            relative: &StrictRelativePath,
            bytes: &[u8],
        ) -> Result<(), SkillStorageError> {
            self.inner.write_file(staging, relative, bytes)
        }

        fn copy_file(
            &self,
            staging: &Path,
            relative: &StrictRelativePath,
            source: &Path,
        ) -> Result<(), SkillStorageError> {
            self.inner.copy_file(staging, relative, source)
        }

        fn write_manifest(&self, staging: &Path, content: &[u8]) -> Result<(), SkillStorageError> {
            self.inner.write_manifest(staging, content)
        }

        fn rollback_create(&self, handle: &CreateHandle) -> Result<(), SkillStorageError> {
            self.inner.rollback_create(handle)
        }

        fn finish_create(&self, handle: &CreateHandle) -> Result<(), SkillStorageError> {
            self.inner.finish_create(handle)
        }

        fn commit_swap(
            &self,
            name: &str,
            from_name: &str,
            skill_id: &SkillId,
            previous_updated_at: Option<i64>,
            staging: &Path,
        ) -> Result<SwapHandle, SkillStorageError> {
            self.inner
                .commit_swap(name, from_name, skill_id, previous_updated_at, staging)
        }

        fn rollback_swap(&self, handle: &SwapHandle) -> Result<(), SkillStorageError> {
            self.inner.rollback_swap(handle)
        }

        fn finish_swap(&self, handle: &SwapHandle) -> Result<(), SkillStorageError> {
            self.inner.finish_swap(handle)
        }

        fn commit_delete(
            &self,
            name: &str,
            skill_id: &SkillId,
        ) -> Result<DeleteHandle, SkillStorageError> {
            self.inner.commit_delete(name, skill_id)
        }

        fn rollback_delete(&self, handle: &DeleteHandle) -> Result<(), SkillStorageError> {
            self.inner.rollback_delete(handle)
        }

        fn finish_delete(&self, handle: &DeleteHandle) -> Result<(), SkillStorageError> {
            self.inner.finish_delete(handle)
        }

        fn formal_exists(&self, name: &str) -> bool {
            self.inner.formal_exists(name)
        }

        fn read_manifest(&self, name: &str) -> Result<Option<Vec<u8>>, SkillStorageError> {
            self.inner.read_manifest(name)
        }

        fn list_formal_names(&self) -> Result<Vec<String>, SkillStorageError> {
            self.inner.list_formal_names()
        }

        fn remove_temp(&self, path: &Path) -> Result<(), SkillStorageError> {
            self.inner.remove_temp(path)
        }

        fn restore_backup(&self, backup: &Path, name: &str) -> Result<(), SkillStorageError> {
            self.inner.restore_backup(backup, name)
        }

        fn remove_dir(&self, path: &Path) -> Result<(), SkillStorageError> {
            self.inner.remove_dir(path)
        }

        fn remove_formal(&self, name: &str) -> Result<(), SkillStorageError> {
            self.inner.remove_formal(name)
        }

        fn list_journals(&self) -> Result<Vec<TransactionJournal>, SkillStorageError> {
            self.inner.list_journals()
        }

        fn remove_journal(&self, journal: &TransactionJournal) -> Result<(), SkillStorageError> {
            self.inner.remove_journal(journal)
        }
    }

    /// Verifies a stalled package promote never blocks unrelated catalog writers.
    ///
    /// Skill creation and import promote the package directory before writing their row, so the
    /// SQLite write lock is never held across the rename. A promote parked indefinitely must
    /// therefore leave concurrent writers free instead of starving them into the 5 second busy
    /// timeout that the pool configures.
    #[test]
    fn stalled_package_promote_leaves_concurrent_catalog_writes_unblocked() {
        with_trace_logging(|| {
            let temp_dir = TempDir::new().unwrap();
            let pool = DatabaseBootstrapper::system()
                .bootstrap_repository_pool(
                    &DatabaseLocation::path(temp_dir.path().join("ora.sqlite3")),
                    &default_migration_catalog().unwrap(),
                )
                .unwrap();
            let skill_repository = SqliteSkillRepository::new(pool.clone());
            let agent_repository = SqliteAgentDefinitionRepository::new(pool);
            let promote = Arc::new(Barrier::new(2));
            let handler = CreateSkillHandler::new(
                skill_repository.clone(),
                PausingSkillStorage {
                    inner: FilesystemSkillStorage::new(
                        temp_dir.path().join("atoms").join("skills"),
                    ),
                    promote: promote.clone(),
                },
                UuidSkillIdGenerator::new(),
                SystemClock,
            );
            let created_agent = AgentDefinition::new(
                AgentDefinitionId::new("agent-1"),
                Namespace::local(),
                "opencode",
                "Assists",
                "",
                AuditFields::new(1, 1, false),
            )
            .unwrap();

            thread::scope(|scope| {
                scope.spawn(|| {
                    handler
                        .handle(CreateSkillRequest {
                            name: "review".to_string(),
                            description: "Reviews changes".to_string(),
                            content: None,
                        })
                        .unwrap();
                });
                scope.spawn(|| {
                    promote.wait();
                    // Runs strictly inside the parked promote: a lock held across the rename
                    // would fail this write with SQLITE_BUSY instead.
                    agent_repository
                        .create_agent_definition(created_agent.clone())
                        .unwrap();
                    promote.wait();
                });
            });

            assert_eq!(
                agent_repository.list_agent_definitions().unwrap(),
                vec![created_agent]
            );
            assert_eq!(
                skill_repository
                    .list_skills()
                    .unwrap()
                    .into_iter()
                    .map(|skill| skill.name)
                    .collect::<Vec<_>>(),
                vec!["review".to_string()]
            );
        });
    }
}
