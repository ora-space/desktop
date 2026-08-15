use ora_application::{
    BACKUP_DIR_NAME, FilesystemSkillStorage, JournalOp, JournalPhase, STAGING_DIR_NAME,
    SkillRepository, SkillStorage, TransactionJournal,
};
use ora_db::{RepositoryPool, SqliteSkillRepository};
use ora_domain::Namespace;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use thiserror::Error;

/// Reports startup reconciliation failures that must block backend readiness.
#[derive(Debug, Error)]
pub enum SkillStorageReconciliationError {
    #[error("skill storage inconsistency for skills: {names:?}")]
    Inconsistent { names: Vec<String> },
    #[error("skill storage reconciliation failed: {message}")]
    OperationFailed { message: String },
}

/// Reconciles formal skill storage with the database before the backend serves requests.
///
/// First interrupted transactions are restored or cleaned from their journal markers, then
/// leftover transaction directories are removed, then orphan formal directories are cleaned
/// and any visible record without its formal directory or root `SKILL.md` blocks startup.
pub(crate) fn reconcile_skill_storage(
    pool: &RepositoryPool,
    skills_root: &Path,
) -> Result<(), SkillStorageReconciliationError> {
    let repository = SqliteSkillRepository::new(pool.clone());
    let storage = FilesystemSkillStorage::new(skills_root.to_path_buf());

    let journals = storage.list_journals().map_err(operation_failed)?;
    for journal in journals {
        recover_journal(&repository, &storage, &journal, skills_root)?;
        storage.remove_journal(&journal).map_err(operation_failed)?;
    }

    cleanup_reserved_transactions(&storage, skills_root)?;

    let visible = repository.list_skills().map_err(operation_failed)?;
    let claimed = visible
        .iter()
        .map(|skill| skill.name.clone())
        .collect::<BTreeSet<_>>();
    let formal_names = storage.list_formal_names().map_err(operation_failed)?;

    let mut inconsistent = Vec::new();
    for name in &claimed {
        let has_directory = storage.formal_exists(name);
        let has_manifest = storage
            .read_manifest(name)
            .map_err(operation_failed)?
            .is_some();
        if !has_directory || !has_manifest {
            inconsistent.push(name.clone());
        }
    }
    if !inconsistent.is_empty() {
        return Err(SkillStorageReconciliationError::Inconsistent {
            names: inconsistent,
        });
    }

    for name in formal_names {
        if !claimed.contains(&name) {
            storage
                .remove_dir(&skills_root.join(&name))
                .map_err(operation_failed)?;
        }
    }
    Ok(())
}

/// Applies one journal marker deterministically based on its phase and the database state.
fn recover_journal(
    repository: &SqliteSkillRepository,
    storage: &FilesystemSkillStorage,
    journal: &TransactionJournal,
    skills_root: &Path,
) -> Result<(), SkillStorageReconciliationError> {
    let staging = Path::new(&journal.staging).to_path_buf();
    let backup = Path::new(&journal.backup).to_path_buf();
    let formal_path = |name: &str| skills_root.join(name);
    let target_visible = repository
        .find_skill_by_name(&Namespace::local(), &journal.name)
        .map_err(operation_failed)?
        .is_some();

    match journal.op {
        JournalOp::Create => match journal.phase {
            JournalPhase::Prepared => {
                // Database was not written; drop the promoted directory and staging.
                if storage.formal_exists(&journal.name) {
                    storage
                        .remove_dir(&formal_path(&journal.name))
                        .map_err(operation_failed)?;
                }
                remove_if_present(storage, &staging)?;
            }
            JournalPhase::Swapped => {
                // Keep the promoted directory only when a database record claims it.
                if !target_visible && storage.formal_exists(&journal.name) {
                    storage
                        .remove_dir(&formal_path(&journal.name))
                        .map_err(operation_failed)?;
                }
            }
        },
        JournalOp::Swap => match journal.phase {
            JournalPhase::Prepared => {
                // The swap never reached its database write; restore the original directory.
                if storage.formal_exists(&journal.name) {
                    storage
                        .remove_dir(&formal_path(&journal.name))
                        .map_err(operation_failed)?;
                }
                if backup.exists() && !storage.formal_exists(&journal.from_name) {
                    storage
                        .restore_backup(&backup, &journal.from_name)
                        .map_err(operation_failed)?;
                }
                remove_if_present(storage, &staging)?;
            }
            JournalPhase::Swapped => {
                let from_visible = repository
                    .find_skill_by_name(&Namespace::local(), &journal.from_name)
                    .map_err(operation_failed)?
                    .is_some();
                if target_visible {
                    // Fully committed; only the compensation backup remains.
                    remove_if_present(storage, &backup)?;
                } else if from_visible {
                    // Database write never happened; restore the original directory.
                    if storage.formal_exists(&journal.name) {
                        storage
                            .remove_dir(&formal_path(&journal.name))
                            .map_err(operation_failed)?;
                    }
                    if backup.exists() && !storage.formal_exists(&journal.from_name) {
                        storage
                            .restore_backup(&backup, &journal.from_name)
                            .map_err(operation_failed)?;
                    }
                } else {
                    // No record claims either name; drop both leftovers.
                    if storage.formal_exists(&journal.name) {
                        storage
                            .remove_dir(&formal_path(&journal.name))
                            .map_err(operation_failed)?;
                    }
                    remove_if_present(storage, &backup)?;
                }
                remove_if_present(storage, &staging)?;
            }
        },
        JournalOp::Delete => match journal.phase {
            JournalPhase::Prepared => {
                // Restore the directory the delete had moved aside.
                if !storage.formal_exists(&journal.name) && backup.exists() {
                    storage
                        .restore_backup(&backup, &journal.name)
                        .map_err(operation_failed)?;
                }
            }
            JournalPhase::Swapped => {
                if target_visible {
                    // The soft delete never committed; restore the directory.
                    if backup.exists() && !storage.formal_exists(&journal.name) {
                        storage
                            .restore_backup(&backup, &journal.name)
                            .map_err(operation_failed)?;
                    }
                } else if backup.exists() {
                    remove_if_present(storage, &backup)?;
                }
            }
        },
    }
    Ok(())
}

/// Removes one reserved directory when it still exists.
fn remove_if_present(
    storage: &FilesystemSkillStorage,
    path: &Path,
) -> Result<(), SkillStorageReconciliationError> {
    if path.exists() {
        storage.remove_temp(path).map_err(operation_failed)?;
    }
    Ok(())
}

/// Removes every leftover transaction directory under the reserved staging and backup roots.
fn cleanup_reserved_transactions(
    storage: &FilesystemSkillStorage,
    skills_root: &Path,
) -> Result<(), SkillStorageReconciliationError> {
    for kind in [STAGING_DIR_NAME, BACKUP_DIR_NAME] {
        let root = skills_root.join(kind);
        if !root.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&root).map_err(operation_failed)? {
            let entry = entry.map_err(operation_failed)?;
            storage
                .remove_temp(&entry.path())
                .map_err(operation_failed)?;
        }
    }
    Ok(())
}

/// Removes leftover Ora-prefixed import directories from OS temporary storage.
///
/// Import sessions are never resumed across restarts, so startup only cleans the
/// `ora-skill-import-*` session snapshots and `ora-skill-import-upload-*` upload staging
/// directories. A previously returned session id resolves to `import_session_expired`.
///
/// A grace period protects actively-written directories: only entries older than
/// [`IMPORT_TEMP_GRACE`] are removed, so a healthy process's in-flight uploads are never
/// swept away and leftovers from a prior run are cleaned on a later startup.
pub(crate) fn cleanup_import_temp_sessions() -> Result<(), SkillStorageReconciliationError> {
    let temp_root = std::env::temp_dir();
    if !temp_root.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(&temp_root).map_err(operation_failed)? {
        let entry = entry.map_err(operation_failed)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("ora-skill-import-") && !name.starts_with("ora-skill-import-upload-") {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| modified.elapsed().is_ok_and(|age| age >= IMPORT_TEMP_GRACE));
        if stale {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
    Ok(())
}

/// Minimum age before an Ora import temp directory is considered stale.
const IMPORT_TEMP_GRACE: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Wraps a reconciliation I/O or port failure into the stable error surface.
fn operation_failed(error: impl std::fmt::Display) -> SkillStorageReconciliationError {
    SkillStorageReconciliationError::OperationFailed {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::reconcile_skill_storage;
    use ora_application::{
        BACKUP_DIR_NAME, FilesystemSkillStorage, NoopSkillImportProgressPublisher,
        SkillImportConfig, SkillImportService, SkillRepository, TransactionJournal,
        UuidSkillImportIdGenerator,
    };
    use ora_contracts::{
        CommitSkillImportRequest, GetSkillImportSessionRequest, PrepareSkillImportRequest,
        SkillImportSource,
    };
    use ora_db::{
        DatabaseBootstrapper, DatabaseLocation, RepositoryPool, SqliteSkillRepository,
        default_migration_catalog,
    };
    use ora_domain::{AuditFields, Namespace, Skill, SkillId};
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    /// Builds a repository pool over one temporary database.
    fn pool(database_path: &Path) -> RepositoryPool {
        DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(database_path),
                &default_migration_catalog().unwrap(),
            )
            .unwrap()
    }

    /// Creates one formal directory with a root manifest on disk.
    fn create_formal(root: &Path, name: &str, body: &str) {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), body).unwrap();
    }

    /// Writes one journal marker at its self-referencing absolute file path.
    fn write_journal(journal: &TransactionJournal) {
        let file = std::path::Path::new(&journal.file);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, serde_json::to_string(journal).unwrap()).unwrap();
        assert!(file.exists());
    }

    #[test]
    fn restores_interrupted_swap_from_backup() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("atoms").join("skills");
        create_formal(&skills_root, "review", "old body");
        let staging = skills_root.join(".ora-staging").join("txn");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("SKILL.md"), "new body").unwrap();
        let backup = skills_root.join(".ora-backup").join("txn");
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("SKILL.md"), "old body").unwrap();

        let journal = TransactionJournal {
            op: ora_application::JournalOp::Swap,
            name: "review".to_string(),
            from_name: "review".to_string(),
            staging: staging.to_string_lossy().into_owned(),
            backup: backup.to_string_lossy().into_owned(),
            phase: ora_application::JournalPhase::Prepared,
            file: skills_root
                .join(".ora-journal")
                .join("txn.json")
                .to_string_lossy()
                .into_owned(),
        };
        write_journal(&journal);

        // The database still claims the old skill.
        let database_path = temp.path().join("ora.sqlite3");
        let repository = SqliteSkillRepository::new(pool(&database_path));
        repository
            .create_skill(
                Skill::new(
                    SkillId::new("skill-1"),
                    Namespace::local(),
                    "review",
                    "Reviews",
                    AuditFields::new(100, 100, false),
                )
                .unwrap(),
            )
            .unwrap();

        reconcile_skill_storage(&pool(&database_path), &skills_root).unwrap();

        assert!(!staging.exists());
        assert_eq!(
            fs::read_to_string(skills_root.join("review").join("SKILL.md")).unwrap(),
            "old body"
        );
        assert!(!backup.exists());
        assert!(!skills_root.join(".ora-journal").join("txn.json").exists());
    }

    /// Verifies a reserved directory name survives a real import followed by startup reconciliation.
    ///
    /// This is the end-to-end shape of the original defect: a skill whose name matched a reserved
    /// transaction directory was promoted onto that directory, and the next startup deleted the
    /// package contents while sweeping what it took to be transaction leftovers. The import must
    /// now refuse the name, leaving reconciliation with nothing to misread.
    #[test]
    fn refuses_reserved_name_import_and_reconciles_cleanly_on_restart() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("atoms").join("skills");
        let database_path = temp.path().join("ora.sqlite3");

        // A legitimate skill shares the tree so reconciliation has real state to preserve.
        create_formal(&skills_root, "review", "---\nname: review\n---\n");
        let repository = SqliteSkillRepository::new(pool(&database_path));
        repository
            .create_skill(
                Skill::new(
                    SkillId::new("skill-1"),
                    Namespace::local(),
                    "review",
                    "Reviews",
                    AuditFields::new(100, 100, false),
                )
                .unwrap(),
            )
            .unwrap();

        let source = temp.path().join("source").join("pkg");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("SKILL.md"),
            format!(
                "---\nname: {BACKUP_DIR_NAME}\ndescription: Collides with reserved\n---\nBody.\n"
            ),
        )
        .unwrap();
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("nested").join("note.md"), "payload").unwrap();

        let service = SkillImportService::new(
            SqliteSkillRepository::new(pool(&database_path)),
            FilesystemSkillStorage::new(skills_root.clone()),
            UuidSkillImportIdGenerator,
            crate::clock::SystemClock,
            NoopSkillImportProgressPublisher,
            SkillImportConfig {
                temp_root: temp.path().join("os-temp"),
                ..SkillImportConfig::default()
            },
        );

        let session = service
            .prepare(PrepareSkillImportRequest {
                source: SkillImportSource::Folder {
                    path: source.parent().unwrap().to_string_lossy().into_owned(),
                },
            })
            .unwrap()
            .session;
        assert_eq!(
            session.candidates[0].status,
            ora_contracts::SkillImportCandidateStatus::Invalid
        );
        assert_eq!(
            session.candidates[0].error_code.as_deref(),
            Some("name_invalid")
        );

        service
            .commit(CommitSkillImportRequest {
                session_id: session.session_id.clone(),
                decisions: vec![],
            })
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let current = service
                .get_session(GetSkillImportSessionRequest {
                    session_id: session.session_id.clone(),
                })
                .unwrap()
                .session;
            if current.status == ora_contracts::SkillImportSessionStatus::Completed {
                assert_eq!(
                    current.progress.results[0].status,
                    ora_contracts::SkillImportResultStatus::Failed
                );
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "commit did not finish"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        // Restart: reconciliation must succeed rather than trip over a package on a reserved root.
        reconcile_skill_storage(&pool(&database_path), &skills_root).unwrap();

        assert_eq!(
            repository
                .find_skill_by_name(&Namespace::local(), BACKUP_DIR_NAME)
                .unwrap(),
            None
        );
        assert_eq!(
            skills_root.join(BACKUP_DIR_NAME).join("SKILL.md").is_file(),
            false
        );
        // The unrelated skill is untouched by the refused import and the sweep.
        assert_eq!(
            fs::read_to_string(skills_root.join("review").join("SKILL.md")).unwrap(),
            "---\nname: review\n---\n"
        );
    }

    #[test]
    fn removes_orphan_formal_directories() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("atoms").join("skills");
        create_formal(&skills_root, "orphan", "no database record");

        reconcile_skill_storage(&pool(&temp.path().join("ora.sqlite3")), &skills_root).unwrap();

        assert!(!skills_root.join("orphan").exists());
    }

    #[test]
    fn blocks_startup_when_visible_record_lacks_formal_directory() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("atoms").join("skills");
        fs::create_dir_all(&skills_root).unwrap();
        let database_path = temp.path().join("ora.sqlite3");
        let repository = SqliteSkillRepository::new(pool(&database_path));
        repository
            .create_skill(
                Skill::new(
                    SkillId::new("skill-1"),
                    Namespace::local(),
                    "missing-dir",
                    "Reviews",
                    AuditFields::new(100, 100, false),
                )
                .unwrap(),
            )
            .unwrap();

        let error = reconcile_skill_storage(&pool(&database_path), &skills_root).unwrap_err();
        assert!(matches!(
            error,
            super::SkillStorageReconciliationError::Inconsistent { names } if names == vec!["missing-dir".to_string()]
        ));
    }

    #[test]
    fn completes_delete_recovery_after_soft_delete() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("atoms").join("skills");
        let database_path = temp.path().join("ora.sqlite3");
        let repository = SqliteSkillRepository::new(pool(&database_path));
        repository
            .create_skill(
                Skill::new(
                    SkillId::new("skill-1"),
                    Namespace::local(),
                    "gone",
                    "Reviews",
                    AuditFields::new(100, 100, false),
                )
                .unwrap(),
            )
            .unwrap();
        repository
            .soft_delete_skill(&SkillId::new("skill-1"), 200)
            .unwrap();

        // Simulate the crash window: the formal directory was moved to backup but the
        // journal was never removed and the soft delete committed.
        let backup = skills_root.join(".ora-backup").join("txn");
        create_formal(&skills_root, "gone", "old body");
        fs::create_dir_all(backup.parent().unwrap()).unwrap();
        fs::rename(skills_root.join("gone"), &backup).unwrap();
        let journal = TransactionJournal {
            op: ora_application::JournalOp::Delete,
            name: "gone".to_string(),
            from_name: "gone".to_string(),
            staging: String::new(),
            backup: backup.to_string_lossy().into_owned(),
            phase: ora_application::JournalPhase::Swapped,
            file: skills_root
                .join(".ora-journal")
                .join("txn.json")
                .to_string_lossy()
                .into_owned(),
        };
        write_journal(&journal);

        reconcile_skill_storage(&pool(&database_path), &skills_root).unwrap();

        // The record is soft-deleted, so recovery cleans the backup instead of restoring it.
        assert!(!backup.exists());
        assert!(!skills_root.join("gone").exists());
    }
}
