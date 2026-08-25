use ora_application::{
    BACKUP_DIR_NAME, FilesystemSkillStorage, JournalOp, JournalPhase, STAGING_DIR_NAME,
    SkillRepository, SkillStorage, TransactionJournal,
};
use ora_db::{RepositoryPool, SourcePublication, SqliteEffectRepository, SqliteSkillRepository};
use ora_domain::SkillId;
use ora_effect::{DesiredSkillState, Digest, SkillName, SkillSource, SkillState, SourceVersion};
use ora_logging::ora_warn;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use thiserror::Error;

/// Reports startup reconciliation failures that must block backend readiness.
///
/// Missing or incomplete packages stay in the catalog as unavailable and never surface
/// here. Only I/O or repository failures during recovery remain fatal.
#[derive(Debug, Error)]
pub enum SkillStorageReconciliationError {
    #[error("skill storage reconciliation failed: {message}")]
    OperationFailed { message: String },
}

/// Reconciles formal skill storage with the database before the backend serves requests.
///
/// First interrupted transactions are restored or cleaned from their journal markers, then
/// leftover transaction directories are removed. Visible records whose formal directory or
/// root `SKILL.md` is missing stay in the catalog as unavailable. Unowned directories
/// without a root `SKILL.md` are removed; untracked complete packages are left in place.
pub(crate) fn reconcile_skill_storage(
    pool: &RepositoryPool,
    skills_root: &Path,
) -> Result<(), SkillStorageReconciliationError> {
    let repository = SqliteSkillRepository::new(pool.clone());
    let effect_repository = SqliteEffectRepository::new(pool.clone());
    let storage = FilesystemSkillStorage::new(skills_root.to_path_buf());

    let journals = storage.list_journals().map_err(operation_failed)?;
    for journal in journals {
        recover_journal(&repository, &storage, &journal, skills_root)?;
        storage.remove_journal(&journal).map_err(operation_failed)?;
    }

    cleanup_reserved_transactions(&storage, skills_root)?;

    let visible = repository.list_skills().map_err(operation_failed)?;
    let mut claimed = BTreeSet::new();
    for skill in visible {
        let has_directory = storage.formal_exists(&skill.name);
        let manifest = storage
            .read_manifest(&skill.name)
            .map_err(operation_failed)?;
        if !has_directory || manifest.is_none() {
            ora_warn!(
                message = "skill package is missing or incomplete; catalog row stays unavailable",
                skill_id = skill.id.to_string(),
                skill_name = skill.name.clone(),
            );
        } else if let Some(manifest) = manifest {
            let parsed = ora_skill_package::parse_manifest(
                &manifest,
                ora_skill_package::Limits::default().max_manifest_bytes,
            );
            if parsed
                .as_ref()
                .is_ok_and(|parsed| parsed.name == skill.name)
            {
                let state = DesiredSkillState::try_new(SkillState {
                    name: SkillName::parse(skill.name.clone()).map_err(operation_failed)?,
                    skill_md_digest: Digest::sha256(&manifest),
                    source: SkillSource::Local {
                        namespace: skill.namespace.clone(),
                        version: SourceVersion::parse(skill.audit_fields.updated_at.to_string())
                            .map_err(operation_failed)?,
                    },
                })
                .map_err(operation_failed)?;
                effect_repository
                    .publish_source(
                        &state,
                        &skills_root.join(&skill.name),
                        SourcePublication::Create,
                        skill.audit_fields.updated_at,
                    )
                    .map_err(operation_failed)?;
            } else {
                ora_warn!(
                    message = "skill package is invalid; source state stays unavailable",
                    skill_id = skill.id.to_string(),
                    skill_name = skill.name.clone(),
                );
            }
        }
        claimed.insert(skill.name);
    }
    let formal_names = storage.list_formal_names().map_err(operation_failed)?;
    for name in formal_names {
        if claimed.contains(&name) {
            continue;
        }
        let has_manifest = storage
            .read_manifest(&name)
            .map_err(operation_failed)?
            .is_some();
        if has_manifest {
            ora_warn!(
                message = "leaving untracked skill package in place",
                skill_name = name.clone(),
            );
            continue;
        }
        storage
            .remove_dir(&skills_root.join(&name))
            .map_err(operation_failed)?;
    }
    Ok(())
}

/// Applies one journal marker deterministically based on its phase and the database state.
///
/// Directory ownership is decided by the immutable id recorded in the journal. A visible row that
/// only shares the user-facing name cannot claim an interrupted transaction's package.
fn recover_journal(
    repository: &SqliteSkillRepository,
    storage: &FilesystemSkillStorage,
    journal: &TransactionJournal,
    skills_root: &Path,
) -> Result<(), SkillStorageReconciliationError> {
    let staging = Path::new(&journal.staging).to_path_buf();
    let backup = Path::new(&journal.backup).to_path_buf();
    let formal_path = |name: &str| skills_root.join(name);
    let owner = repository
        .find_skill(&SkillId::new(&journal.skill_id))
        .map_err(operation_failed)?;

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
                // A same-named unrelated row cannot keep this interrupted create.
                if owner.is_none() && storage.formal_exists(&journal.name) {
                    storage
                        .remove_dir(&formal_path(&journal.name))
                        .map_err(operation_failed)?;
                }
            }
        },
        JournalOp::Swap {
            previous_updated_at,
        } => match journal.phase {
            JournalPhase::Prepared => {
                // The swap never reached its database write; restore the original directory.
                restore_interrupted_swap(storage, journal, &backup, skills_root)?;
                remove_if_present(storage, &staging)?;
            }
            JournalPhase::Swapped => {
                let owner_name = owner.as_ref().map(|skill| skill.name.as_str());
                let database_write_pending = previous_updated_at.is_some_and(|previous| {
                    owner.as_ref().is_some_and(|skill| {
                        skill.name == journal.from_name && skill.audit_fields.updated_at == previous
                    })
                });

                if database_write_pending {
                    // The exact pre-swap database version remains, so restore its package.
                    restore_interrupted_swap(storage, journal, &backup, skills_root)?;
                } else if owner_name == Some(journal.name.as_str()) {
                    // The target version committed, or a later update superseded this journal.
                    remove_if_present(storage, &backup)?;
                } else if previous_updated_at.is_none() && owner.is_none() {
                    // A new row failed to claim an existing untracked package. Restore that
                    // package instead of losing it; a same-named unrelated row is not the owner.
                    restore_interrupted_swap(storage, journal, &backup, skills_root)?;
                } else {
                    // The journaled owner no longer claims either path; discard transaction
                    // leftovers without allowing a same-named unrelated row to inherit them.
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
                if owner.is_some() {
                    // The soft delete never committed; restore the journal owner's directory.
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

/// Restores the pre-swap package after recovery proves the database write never committed.
fn restore_interrupted_swap(
    storage: &FilesystemSkillStorage,
    journal: &TransactionJournal,
    backup: &Path,
    skills_root: &Path,
) -> Result<(), SkillStorageReconciliationError> {
    if storage.formal_exists(&journal.name) {
        storage
            .remove_dir(&skills_root.join(&journal.name))
            .map_err(operation_failed)?;
    }
    if backup.exists() && !storage.formal_exists(&journal.from_name) {
        storage
            .restore_backup(backup, &journal.from_name)
            .map_err(operation_failed)?;
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
            op: ora_application::JournalOp::Swap {
                previous_updated_at: Some(100),
            },
            skill_id: "skill-1".to_string(),
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

    #[test]
    fn restores_same_name_swap_when_database_write_did_not_commit() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("atoms").join("skills");
        let database_path = temp.path().join("ora.sqlite3");
        let repository = SqliteSkillRepository::new(pool(&database_path));
        repository
            .create_skill(
                Skill::new(
                    SkillId::new("skill-1"),
                    Namespace::local(),
                    "review",
                    "Old description",
                    AuditFields::new(100, 100, false),
                )
                .unwrap(),
            )
            .unwrap();

        create_formal(&skills_root, "review", "new body");
        let backup = skills_root.join(".ora-backup").join("txn");
        create_formal(&skills_root.join(".ora-backup"), "txn", "old body");
        let journal = TransactionJournal {
            op: ora_application::JournalOp::Swap {
                previous_updated_at: Some(100),
            },
            skill_id: "skill-1".to_string(),
            name: "review".to_string(),
            from_name: "review".to_string(),
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

        assert_eq!(
            fs::read_to_string(skills_root.join("review").join("SKILL.md")).unwrap(),
            "old body"
        );
        assert!(!backup.exists());
        assert!(!Path::new(&journal.file).exists());
    }

    #[test]
    fn keeps_same_name_swap_when_database_write_committed() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("atoms").join("skills");
        let database_path = temp.path().join("ora.sqlite3");
        let repository = SqliteSkillRepository::new(pool(&database_path));
        repository
            .create_skill(
                Skill::new(
                    SkillId::new("skill-1"),
                    Namespace::local(),
                    "review",
                    "New description",
                    AuditFields::new(100, 200, false),
                )
                .unwrap(),
            )
            .unwrap();

        create_formal(&skills_root, "review", "new body");
        let backup = skills_root.join(".ora-backup").join("txn");
        create_formal(&skills_root.join(".ora-backup"), "txn", "old body");
        let journal = TransactionJournal {
            op: ora_application::JournalOp::Swap {
                previous_updated_at: Some(100),
            },
            skill_id: "skill-1".to_string(),
            name: "review".to_string(),
            from_name: "review".to_string(),
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

        assert_eq!(
            fs::read_to_string(skills_root.join("review").join("SKILL.md")).unwrap(),
            "new body"
        );
        assert!(!backup.exists());
        assert!(!Path::new(&journal.file).exists());
    }

    #[test]
    fn delete_recovery_does_not_restore_backup_for_unrelated_same_name_skill() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("atoms").join("skills");
        let database_path = temp.path().join("ora.sqlite3");
        let repository = SqliteSkillRepository::new(pool(&database_path));
        repository
            .create_skill(
                Skill::new(
                    SkillId::new("skill-unrelated"),
                    Namespace::local(),
                    "gone",
                    "Unrelated",
                    AuditFields::new(300, 300, false),
                )
                .unwrap(),
            )
            .unwrap();

        create_formal(&skills_root, "gone", "unrelated body");
        let backup = skills_root.join(".ora-backup").join("txn");
        create_formal(
            &skills_root.join(".ora-backup"),
            "txn",
            "deleted owner body",
        );
        let journal = TransactionJournal {
            op: ora_application::JournalOp::Delete,
            skill_id: "skill-deleted".to_string(),
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

        assert_eq!(
            fs::read_to_string(skills_root.join("gone").join("SKILL.md")).unwrap(),
            "unrelated body"
        );
        assert!(!backup.exists());
        assert!(!Path::new(&journal.file).exists());
    }

    #[test]
    fn removes_interrupted_create_not_owned_by_same_name_database_skill() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("atoms").join("skills");
        let database_path = temp.path().join("ora.sqlite3");
        let repository = SqliteSkillRepository::new(pool(&database_path));
        repository
            .create_skill(
                Skill::new(
                    SkillId::new("skill-db-only"),
                    Namespace::local(),
                    "review",
                    "Pre-existing database row",
                    AuditFields::new(100, 100, false),
                )
                .unwrap(),
            )
            .unwrap();

        // Crash window: import promoted `review/` for a different skill id, then died
        // before inserting that row. A same-named visible row must not inherit it.
        create_formal(&skills_root, "review", "crashed import body");
        let journal = TransactionJournal {
            op: ora_application::JournalOp::Create,
            skill_id: "skill-import".to_string(),
            name: "review".to_string(),
            from_name: "review".to_string(),
            staging: String::new(),
            backup: String::new(),
            phase: ora_application::JournalPhase::Swapped,
            file: skills_root
                .join(".ora-journal")
                .join("txn.json")
                .to_string_lossy()
                .into_owned(),
        };
        write_journal(&journal);

        ora_logging::with_trace_logging(|| {
            reconcile_skill_storage(&pool(&database_path), &skills_root).unwrap();
        });

        assert!(!skills_root.join("review").exists());
        assert!(!Path::new(&journal.file).exists());
        assert!(
            repository
                .find_skill(&SkillId::new("skill-db-only"))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn keeps_promoted_create_when_journal_skill_id_exists() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("atoms").join("skills");
        let database_path = temp.path().join("ora.sqlite3");
        let repository = SqliteSkillRepository::new(pool(&database_path));
        repository
            .create_skill(
                Skill::new(
                    SkillId::new("skill-import"),
                    Namespace::local(),
                    "review",
                    "Imported",
                    AuditFields::new(100, 100, false),
                )
                .unwrap(),
            )
            .unwrap();
        create_formal(&skills_root, "review", "imported body");
        let journal = TransactionJournal {
            op: ora_application::JournalOp::Create,
            skill_id: "skill-import".to_string(),
            name: "review".to_string(),
            from_name: "review".to_string(),
            staging: String::new(),
            backup: String::new(),
            phase: ora_application::JournalPhase::Swapped,
            file: skills_root
                .join(".ora-journal")
                .join("txn.json")
                .to_string_lossy()
                .into_owned(),
        };
        write_journal(&journal);

        reconcile_skill_storage(&pool(&database_path), &skills_root).unwrap();

        assert_eq!(
            fs::read_to_string(skills_root.join("review").join("SKILL.md")).unwrap(),
            "imported body"
        );
        assert!(!Path::new(&journal.file).exists());
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
    fn removes_untracked_directories_without_a_manifest() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("atoms").join("skills");
        fs::create_dir_all(skills_root.join("orphan")).unwrap();

        reconcile_skill_storage(&pool(&temp.path().join("ora.sqlite3")), &skills_root).unwrap();

        assert!(!skills_root.join("orphan").exists());
    }

    #[test]
    fn restores_untracked_package_after_interrupted_claim_swap() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("atoms").join("skills");
        create_formal(&skills_root, "stray", "untracked body");
        let staging = skills_root.join(".ora-staging").join("txn");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("SKILL.md"), "new body").unwrap();
        let backup = skills_root.join(".ora-backup").join("txn");
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("SKILL.md"), "untracked body").unwrap();
        fs::remove_dir_all(skills_root.join("stray")).unwrap();
        create_formal(&skills_root, "stray", "new body");

        let journal = TransactionJournal {
            op: ora_application::JournalOp::Swap {
                previous_updated_at: None,
            },
            skill_id: "skill-import".to_string(),
            name: "stray".to_string(),
            from_name: "stray".to_string(),
            staging: staging.to_string_lossy().into_owned(),
            backup: backup.to_string_lossy().into_owned(),
            phase: ora_application::JournalPhase::Swapped,
            file: skills_root
                .join(".ora-journal")
                .join("txn.json")
                .to_string_lossy()
                .into_owned(),
        };
        write_journal(&journal);

        reconcile_skill_storage(&pool(&temp.path().join("ora.sqlite3")), &skills_root).unwrap();

        assert_eq!(
            fs::read_to_string(skills_root.join("stray").join("SKILL.md")).unwrap(),
            "untracked body"
        );
        assert!(!backup.exists());
        assert!(!staging.exists());
        assert!(!skills_root.join(".ora-journal").join("txn.json").exists());
    }

    #[test]
    fn keeps_untracked_packages_that_still_have_a_manifest() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("atoms").join("skills");
        create_formal(&skills_root, "stray", "untracked");

        ora_logging::with_trace_logging(|| {
            reconcile_skill_storage(&pool(&temp.path().join("ora.sqlite3")), &skills_root).unwrap();
        });

        assert!(skills_root.join("stray").join("SKILL.md").is_file());
    }

    #[test]
    fn keeps_visible_record_when_formal_directory_is_missing() {
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

        ora_logging::with_trace_logging(|| {
            reconcile_skill_storage(&pool(&database_path), &skills_root).unwrap();
        });

        assert!(
            repository
                .find_skill_by_name(&Namespace::local(), "missing-dir")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn keeps_visible_record_when_root_manifest_is_missing() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("atoms").join("skills");
        let leftover = skills_root.join("broken");
        fs::create_dir_all(&leftover).unwrap();
        fs::write(leftover.join("notes.md"), "not a manifest").unwrap();
        let database_path = temp.path().join("ora.sqlite3");
        let repository = SqliteSkillRepository::new(pool(&database_path));
        repository
            .create_skill(
                Skill::new(
                    SkillId::new("skill-1"),
                    Namespace::local(),
                    "broken",
                    "Reviews",
                    AuditFields::new(100, 100, false),
                )
                .unwrap(),
            )
            .unwrap();

        ora_logging::with_trace_logging(|| {
            reconcile_skill_storage(&pool(&database_path), &skills_root).unwrap();
        });

        assert!(
            repository
                .find_skill_by_name(&Namespace::local(), "broken")
                .unwrap()
                .is_some()
        );
        assert!(leftover.exists());
    }

    #[test]
    fn keeps_visible_record_when_root_manifest_is_damaged() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("atoms").join("skills");
        let leftover = skills_root.join("damaged");
        fs::create_dir_all(&leftover).unwrap();
        fs::write(leftover.join("SKILL.md"), "---\nname: [unterminated").unwrap();
        let database_path = temp.path().join("ora.sqlite3");
        let repository = SqliteSkillRepository::new(pool(&database_path));
        repository
            .create_skill(
                Skill::new(
                    SkillId::new("skill-2"),
                    Namespace::local(),
                    "damaged",
                    "Reviews",
                    AuditFields::new(100, 100, false),
                )
                .unwrap(),
            )
            .unwrap();

        ora_logging::with_trace_logging(|| {
            reconcile_skill_storage(&pool(&database_path), &skills_root).unwrap();
        });

        assert!(
            repository
                .find_skill_by_name(&Namespace::local(), "damaged")
                .unwrap()
                .is_some()
        );
        assert!(leftover.exists());
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
            skill_id: "skill-1".to_string(),
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
