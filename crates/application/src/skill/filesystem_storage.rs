use super::storage::{
    BACKUP_DIR_NAME, CreateHandle, DeleteHandle, JOURNAL_DIR_NAME, JournalOp, JournalPhase,
    STAGING_DIR_NAME, SkillStorage, SkillStorageError, SwapHandle, TransactionJournal,
};
use ora_domain::SkillId;
use ora_utils::path::StrictRelativePath;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Default filesystem implementation of [`SkillStorage`] rooted at the formal skills tree.
///
/// All transaction artifacts live under `<skills_root>/<reserved>/` so renames stay on the
/// same filesystem and startup recovery can deterministically resolve interrupted mutations.
/// Journal writes and directory promotes flush metadata before returning so a later database
/// commit cannot land without a durable package directory on platforms that support it.
#[derive(Debug, Clone)]
pub struct FilesystemSkillStorage {
    skills_root: PathBuf,
    #[cfg(test)]
    fail_next_journal_phase_update: Arc<AtomicBool>,
}

/// Selects whether a recursive package copy participates in a durable mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CopyDurability {
    Durable,
    Transient,
}

impl FilesystemSkillStorage {
    /// Builds storage rooted at the formal skill directory parent.
    pub fn new(skills_root: PathBuf) -> Self {
        Self {
            skills_root,
            #[cfg(test)]
            fail_next_journal_phase_update: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Injects one journal phase-write failure for rollback tests.
    #[cfg(test)]
    fn fail_next_journal_phase_update(&self) {
        self.fail_next_journal_phase_update
            .store(true, Ordering::SeqCst);
    }

    /// Returns the formal directory for one skill name.
    fn formal_path(&self, name: &str) -> PathBuf {
        self.skills_root.join(name)
    }

    /// Returns the reserved root used for one transaction artifact kind.
    fn reserved_root(&self, kind: &str) -> PathBuf {
        self.skills_root.join(kind)
    }

    /// Copies the entire formal package of one skill into `destination`, overwriting existing files.
    ///
    /// Used by workflow deployment to materialize skills into capability-selected Agent discovery
    /// directories. `destination` is created when missing.
    pub fn copy_package_to(&self, name: &str, destination: &Path) -> Result<(), SkillStorageError> {
        let source = self.formal_path(name);
        if !source.is_dir() {
            return Err(SkillStorageError::FormalDirectoryMissing {
                name: name.to_string(),
            });
        }
        fs::create_dir_all(destination).map_err(map_storage_error)?;
        copy_dir_contents(&source, destination, CopyDurability::Transient).map_err(|source| {
            SkillStorageError::OperationFailed {
                message: format!(
                    "failed to copy skill {name} to {}: {source}",
                    destination.display()
                ),
            }
        })
    }

    /// Writes a journal marker with the current phase, ensuring the journal root exists.
    fn write_journal(&self, journal: &TransactionJournal) -> Result<(), SkillStorageError> {
        if let Some(parent) = Path::new(&journal.file).parent() {
            fs::create_dir_all(parent).map_err(map_storage_error)?;
        }
        let payload =
            serde_json::to_string(journal).map_err(|error| SkillStorageError::OperationFailed {
                message: format!("failed to serialize transaction journal: {error}"),
            })?;
        fs::write(&journal.file, payload).map_err(map_storage_error)?;
        // Journals must hit disk before the filesystem swap so power-loss recovery can still
        // distinguish Prepared from Swapped after the process is gone. A flush failure must
        // leave the written marker in place: callers may already have renamed directories,
        // and deleting the journal would let startup backup cleanup destroy the original package.
        super::durability::persist_file(Path::new(&journal.file)).map_err(map_storage_error)
    }

    /// Builds a journal marker for one transaction with deterministic paths.
    fn new_journal(
        &self,
        op: JournalOp,
        skill_id: &SkillId,
        name: &str,
        from_name: &str,
        staging: Option<&Path>,
        backup: Option<&Path>,
    ) -> TransactionJournal {
        let journal_root = self.reserved_root(JOURNAL_DIR_NAME);
        let transaction_id = staging
            .and_then(Path::file_name)
            .map_or_else(new_transaction_id, |name| {
                name.to_string_lossy().into_owned()
            });
        TransactionJournal {
            op,
            skill_id: skill_id.to_string(),
            name: name.to_string(),
            from_name: from_name.to_string(),
            staging: staging.map_or_else(String::new, |path| path.to_string_lossy().into_owned()),
            backup: backup.map_or_else(String::new, |path| path.to_string_lossy().into_owned()),
            phase: JournalPhase::Prepared,
            file: journal_root
                .join(format!("{transaction_id}.json"))
                .to_string_lossy()
                .into_owned(),
        }
    }

    /// Updates a journal marker's phase in place.
    fn update_journal_phase(
        &self,
        journal: &mut TransactionJournal,
        phase: JournalPhase,
    ) -> Result<(), SkillStorageError> {
        journal.phase = phase;
        #[cfg(test)]
        if self
            .fail_next_journal_phase_update
            .swap(false, Ordering::SeqCst)
        {
            return Err(SkillStorageError::OperationFailed {
                message: "injected journal phase update failure".to_string(),
            });
        }
        self.write_journal(journal)
    }
}

impl SkillStorage for FilesystemSkillStorage {
    fn create_staging(&self) -> Result<PathBuf, SkillStorageError> {
        let root = self.reserved_root(STAGING_DIR_NAME);
        fs::create_dir_all(&root).map_err(map_storage_error)?;
        let staging = root.join(new_transaction_id());
        fs::create_dir_all(&staging).map_err(map_storage_error)?;
        Ok(staging)
    }

    fn stage_existing(&self, name: &str, staging: &Path) -> Result<(), SkillStorageError> {
        let source = self.formal_path(name);
        if !source.is_dir() {
            return Err(SkillStorageError::FormalDirectoryMissing {
                name: name.to_string(),
            });
        }
        copy_dir_contents(&source, staging, CopyDurability::Durable).map_err(map_storage_error)
    }

    fn write_file(
        &self,
        staging: &Path,
        relative: &StrictRelativePath,
        bytes: &[u8],
    ) -> Result<(), SkillStorageError> {
        let destination = relative.to_path(staging);
        let parent = destination
            .parent()
            .ok_or_else(|| SkillStorageError::OperationFailed {
                message: "staging file path has no parent".to_string(),
            })?;
        fs::create_dir_all(parent).map_err(map_storage_error)?;
        fs::write(&destination, bytes).map_err(map_storage_error)?;
        super::durability::persist_package_file(&destination, staging).map_err(map_storage_error)
    }

    fn copy_file(
        &self,
        staging: &Path,
        relative: &StrictRelativePath,
        source: &Path,
    ) -> Result<(), SkillStorageError> {
        let destination = relative.to_path(staging);
        let parent = destination
            .parent()
            .ok_or_else(|| SkillStorageError::OperationFailed {
                message: "staging file path has no parent".to_string(),
            })?;
        fs::create_dir_all(parent).map_err(map_storage_error)?;
        {
            let mut input = fs::File::open(source).map_err(map_storage_error)?;
            let mut output = fs::File::create(&destination).map_err(map_storage_error)?;
            std::io::copy(&mut input, &mut output).map_err(map_storage_error)?;
        }
        super::durability::persist_package_file(&destination, staging).map_err(map_storage_error)
    }

    fn write_manifest(&self, staging: &Path, content: &[u8]) -> Result<(), SkillStorageError> {
        let manifest = staging.join("SKILL.md");
        fs::write(&manifest, content).map_err(map_storage_error)?;
        super::durability::persist_package_file(&manifest, staging).map_err(map_storage_error)
    }

    fn commit_create(
        &self,
        name: &str,
        skill_id: &SkillId,
        staging: &Path,
    ) -> Result<CreateHandle, SkillStorageError> {
        let formal = self.formal_path(name);
        if formal.exists() {
            return Err(SkillStorageError::FormalDirectoryExists {
                name: name.to_string(),
            });
        }
        let mut journal =
            self.new_journal(JournalOp::Create, skill_id, name, name, Some(staging), None);
        self.write_journal(&journal)?;
        if let Err(error) = rename_persistently(staging, &formal) {
            abandon_journal_after_rename_failure(&journal.file, staging, &formal);
            return Err(map_promote_error(error, name));
        }
        if let Err(error) = self.update_journal_phase(&mut journal, JournalPhase::Swapped) {
            if fs::rename(&formal, staging).is_ok() {
                let _ = fs::remove_file(&journal.file);
            }
            return Err(error);
        }
        Ok(CreateHandle {
            name: name.to_string(),
            staging: staging.to_path_buf(),
            journal: PathBuf::from(&journal.file),
        })
    }

    fn rollback_create(&self, handle: &CreateHandle) -> Result<(), SkillStorageError> {
        if self.formal_path(&handle.name).exists() {
            fs::remove_dir_all(self.formal_path(&handle.name)).map_err(map_storage_error)?;
        }
        let _ = fs::remove_file(&handle.journal);
        if handle.staging.exists() {
            fs::remove_dir_all(&handle.staging).map_err(map_storage_error)?;
        }
        Ok(())
    }

    fn finish_create(&self, handle: &CreateHandle) -> Result<(), SkillStorageError> {
        best_effort_cleanup(&handle.journal, &handle.staging);
        Ok(())
    }

    fn commit_swap(
        &self,
        name: &str,
        from_name: &str,
        skill_id: &SkillId,
        previous_updated_at: Option<i64>,
        staging: &Path,
    ) -> Result<SwapHandle, SkillStorageError> {
        let target_formal = self.formal_path(name);
        if name != from_name && target_formal.exists() {
            return Err(SkillStorageError::FormalDirectoryExists {
                name: name.to_string(),
            });
        }
        let from_formal = self.formal_path(from_name);
        if !from_formal.exists() {
            return Err(SkillStorageError::FormalDirectoryMissing {
                name: from_name.to_string(),
            });
        }
        let backup = self
            .reserved_root(BACKUP_DIR_NAME)
            .join(new_transaction_id());
        let mut journal = self.new_journal(
            JournalOp::Swap {
                previous_updated_at,
            },
            skill_id,
            name,
            from_name,
            Some(staging),
            Some(&backup),
        );
        self.write_journal(&journal)?;
        if let Some(parent) = backup.parent() {
            fs::create_dir_all(parent).map_err(map_storage_error)?;
        }

        if let Err(error) = rename_persistently(&from_formal, &backup) {
            abandon_journal_after_rename_failure(&journal.file, &from_formal, &backup);
            return Err(map_storage_error(error));
        }
        if let Err(error) = rename_persistently(staging, &target_formal) {
            restore_backup_or_keep_journal(&backup, &from_formal, &journal.file);
            return Err(map_promote_error(error, name));
        }
        if let Err(error) = self.update_journal_phase(&mut journal, JournalPhase::Swapped) {
            let _ = fs::remove_dir_all(&target_formal);
            restore_backup_or_keep_journal(&backup, &from_formal, &journal.file);
            return Err(error);
        }
        Ok(SwapHandle {
            name: name.to_string(),
            from_name: from_name.to_string(),
            staging: staging.to_path_buf(),
            backup,
            journal: PathBuf::from(&journal.file),
        })
    }

    fn rollback_swap(&self, handle: &SwapHandle) -> Result<(), SkillStorageError> {
        let target_formal = self.formal_path(&handle.name);
        if target_formal.exists() {
            fs::remove_dir_all(&target_formal).map_err(map_storage_error)?;
        }
        let from_formal = self.formal_path(&handle.from_name);
        if handle.backup.exists() && !from_formal.exists() {
            fs::rename(&handle.backup, &from_formal).map_err(map_storage_error)?;
        }
        let _ = fs::remove_file(&handle.journal);
        if handle.staging.exists() {
            fs::remove_dir_all(&handle.staging).map_err(map_storage_error)?;
        }
        Ok(())
    }

    fn finish_swap(&self, handle: &SwapHandle) -> Result<(), SkillStorageError> {
        best_effort_cleanup(&handle.journal, &handle.backup);
        Ok(())
    }

    fn commit_delete(
        &self,
        name: &str,
        skill_id: &SkillId,
    ) -> Result<DeleteHandle, SkillStorageError> {
        let formal = self.formal_path(name);
        if !formal.exists() {
            return Err(SkillStorageError::FormalDirectoryMissing {
                name: name.to_string(),
            });
        }
        let backup = self
            .reserved_root(BACKUP_DIR_NAME)
            .join(new_transaction_id());
        let mut journal =
            self.new_journal(JournalOp::Delete, skill_id, name, name, None, Some(&backup));
        self.write_journal(&journal)?;
        if let Some(parent) = backup.parent() {
            fs::create_dir_all(parent).map_err(map_storage_error)?;
        }
        if let Err(error) = rename_persistently(&formal, &backup) {
            abandon_journal_after_rename_failure(&journal.file, &formal, &backup);
            return Err(map_storage_error(error));
        }
        if let Err(error) = self.update_journal_phase(&mut journal, JournalPhase::Swapped) {
            restore_backup_or_keep_journal(&backup, &formal, &journal.file);
            return Err(error);
        }
        Ok(DeleteHandle {
            name: name.to_string(),
            backup,
            journal: PathBuf::from(&journal.file),
        })
    }

    fn rollback_delete(&self, handle: &DeleteHandle) -> Result<(), SkillStorageError> {
        let formal = self.formal_path(&handle.name);
        if handle.backup.exists() && !formal.exists() {
            fs::rename(&handle.backup, &formal).map_err(map_storage_error)?;
        }
        let _ = fs::remove_file(&handle.journal);
        Ok(())
    }

    fn finish_delete(&self, handle: &DeleteHandle) -> Result<(), SkillStorageError> {
        best_effort_cleanup(&handle.journal, &handle.backup);
        Ok(())
    }

    fn formal_exists(&self, name: &str) -> bool {
        self.formal_path(name).is_dir()
    }

    fn read_manifest(&self, name: &str) -> Result<Option<Vec<u8>>, SkillStorageError> {
        let manifest = self.formal_path(name).join("SKILL.md");
        if !manifest.is_file() {
            return Ok(None);
        }
        fs::read(&manifest).map(Some).map_err(map_storage_error)
    }

    fn list_formal_names(&self) -> Result<Vec<String>, SkillStorageError> {
        let mut names = Vec::new();
        if !self.skills_root.is_dir() {
            return Ok(names);
        }
        for entry in fs::read_dir(&self.skills_root).map_err(map_storage_error)? {
            let entry = entry.map_err(map_storage_error)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            // Skipping dot-prefixed entries excludes the reserved transaction roots. It stays
            // deliberately broader than those three names so an unrelated hidden entry is never
            // reported as a formal skill and deleted as an orphan by startup reconciliation.
            if name.starts_with('.') {
                continue;
            }
            if entry.file_type().map_err(map_storage_error)?.is_dir() {
                names.push(name);
            }
        }
        names.sort();
        Ok(names)
    }

    fn remove_temp(&self, path: &Path) -> Result<(), SkillStorageError> {
        fs::remove_dir_all(path).map_err(map_storage_error)
    }

    fn restore_backup(&self, backup: &Path, name: &str) -> Result<(), SkillStorageError> {
        super::durability::rename_directory_for_recovery(backup, &self.formal_path(name))
            .map_err(map_storage_error)
    }

    fn remove_dir(&self, path: &Path) -> Result<(), SkillStorageError> {
        fs::remove_dir_all(path).map_err(map_storage_error)
    }

    fn remove_formal(&self, name: &str) -> Result<(), SkillStorageError> {
        let path = self.formal_path(name);
        if path.exists() {
            fs::remove_dir_all(&path).map_err(map_storage_error)?;
        }
        Ok(())
    }

    fn list_journals(&self) -> Result<Vec<TransactionJournal>, SkillStorageError> {
        let root = self.reserved_root(JOURNAL_DIR_NAME);
        let mut journals = Vec::new();
        if !root.is_dir() {
            return Ok(journals);
        }
        for entry in fs::read_dir(&root).map_err(map_storage_error)? {
            let entry = entry.map_err(map_storage_error)?;
            if entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                let payload = fs::read_to_string(entry.path()).map_err(map_storage_error)?;
                if let Ok(journal) = serde_json::from_str::<TransactionJournal>(&payload) {
                    journals.push(journal);
                } else {
                    ora_logging::ora_warn!(
                        message = "ignoring malformed skill transaction journal",
                        journal_path = %entry.path().display(),
                    );
                }
            }
        }
        journals.sort_by(|left, right| left.file.cmp(&right.file));
        Ok(journals)
    }

    fn remove_journal(&self, journal: &TransactionJournal) -> Result<(), SkillStorageError> {
        fs::remove_file(&journal.file).map_err(map_storage_error)
    }
}

/// Produces one unique transaction identifier from the shared UUID generator.
fn new_transaction_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Removes one journal marker and one leftover transaction directory after a committed mutation.
///
/// Post-commit cleanup must never turn a successful database-and-filesystem mutation into a
/// user-visible failure; leftovers are reclaimed by startup reconciliation instead.
fn best_effort_cleanup(journal: &Path, leftover: &Path) {
    if let Err(error) = fs::remove_file(journal) {
        ora_logging::ora_warn!(
            message = "failed to remove a skill transaction journal after commit",
            journal_path = %journal.display(),
            error = %error,
        );
    }
    if leftover.exists()
        && let Err(error) = fs::remove_dir_all(leftover)
    {
        ora_logging::ora_warn!(
            message = "failed to remove a skill transaction leftover after commit",
            leftover_path = %leftover.display(),
            error = %error,
        );
    }
}

/// Recursively copies one directory's regular files into a destination directory.
///
/// Symbolic links and special files are not recreated. Files are written through a fresh
/// `File::create` so the destination gets application-defined default permissions rather than
/// inheriting the source's ownership, mode, or timestamps (spec: no metadata preservation).
fn copy_dir_contents(
    source: &Path,
    destination: &Path,
    durability: CopyDurability,
) -> std::io::Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            fs::create_dir_all(&destination_path)?;
            copy_dir_contents(&source_path, &destination_path, durability)?;
            if durability == CopyDurability::Durable {
                super::durability::sync_directory(&destination_path)?;
            }
        } else if file_type.is_file() {
            {
                let mut input = fs::File::open(&source_path)?;
                let mut output = fs::File::create(&destination_path)?;
                std::io::copy(&mut input, &mut output)?;
            }
            if durability == CopyDurability::Durable {
                super::durability::persist_file(&destination_path)?;
            }
        }
    }
    Ok(())
}

/// Promotes or restores a directory and flushes parent metadata before the caller continues.
fn rename_persistently(from: &Path, to: &Path) -> io::Result<()> {
    super::durability::rename_directory_persistently(from, to)
}

/// Drops a journal marker unless a failed durable rename left the source at its destination.
///
/// A pre-existing destination can make both paths exist after a rename failure. That is an
/// occupancy conflict, not a partial promote, so retaining a Prepared journal would let startup
/// recovery delete the directory that won the race. The marker remains only when the source is
/// gone and the destination exists, which means the rename happened but its compensation failed.
fn abandon_journal_after_rename_failure(
    journal: impl AsRef<Path>,
    source: &Path,
    destination: &Path,
) {
    if source.exists() || !destination.exists() {
        let _ = fs::remove_file(journal);
    }
}

/// Restores a compensation backup and drops the journal only when the original path is back
/// and the backup is gone.
///
/// If the formal path already has a directory but the backup is still present (a failed
/// overwrite undo), the journal must stay: startup recovery uses it to put the original
/// package back. Dropping the marker would let leftover-backup cleanup destroy that copy.
fn restore_backup_or_keep_journal(backup: &Path, original: &Path, journal: impl AsRef<Path>) {
    if !original.exists() {
        let _ = fs::rename(backup, original);
    }
    if original.exists() && !backup.exists() {
        let _ = fs::remove_file(journal);
    }
}

/// Converts filesystem failures into stable storage-port errors.
fn map_storage_error(error: io::Error) -> SkillStorageError {
    SkillStorageError::OperationFailed {
        message: error.to_string(),
    }
}

/// Maps a promotion rename failure, keeping destination occupancy typed.
///
/// `exists()` and `rename` are not atomic. A concurrent promoter can create the
/// destination in that window; Unix then typically reports `DirectoryNotEmpty` and
/// Windows `AlreadyExists`. Those kinds stay `FormalDirectoryExists` so callers can
/// project a conflict instead of an internal storage failure.
fn map_promote_error(error: io::Error, name: &str) -> SkillStorageError {
    match error.kind() {
        io::ErrorKind::AlreadyExists | io::ErrorKind::DirectoryNotEmpty => {
            SkillStorageError::FormalDirectoryExists {
                name: name.to_string(),
            }
        }
        _ => map_storage_error(error),
    }
}

#[cfg(test)]
mod tests {
    use super::{FilesystemSkillStorage, map_promote_error};
    use crate::skill::{JOURNAL_DIR_NAME, SkillStorage, SkillStorageError};
    use ora_domain::SkillId;
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::io;
    use tempfile::TempDir;

    /// Creates one storage root and its formal parent for transaction tests.
    fn storage(temp: &TempDir) -> FilesystemSkillStorage {
        let root = temp.path().join("skills");
        fs::create_dir_all(&root).unwrap();
        FilesystemSkillStorage::new(root)
    }

    #[test]
    fn commit_create_returns_typed_conflict_when_destination_exists() {
        let temp = TempDir::new().unwrap();
        let storage = FilesystemSkillStorage::new(temp.path().to_path_buf());
        let destination = temp.path().join("grilling");
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join("SKILL.md"), "existing").unwrap();
        let staging = storage.create_staging().unwrap();
        fs::write(staging.join("SKILL.md"), "incoming").unwrap();

        assert_eq!(
            storage
                .commit_create("grilling", &SkillId::new("skill-1"), &staging)
                .unwrap_err(),
            SkillStorageError::FormalDirectoryExists {
                name: "grilling".to_string(),
            }
        );
        assert!(staging.exists());
    }

    #[test]
    fn maps_destination_occupied_rename_kinds_to_typed_conflict() {
        assert_eq!(
            map_promote_error(
                io::Error::new(io::ErrorKind::AlreadyExists, "exists"),
                "grilling",
            ),
            SkillStorageError::FormalDirectoryExists {
                name: "grilling".to_string(),
            }
        );
        assert_eq!(
            map_promote_error(
                io::Error::new(io::ErrorKind::DirectoryNotEmpty, "not empty"),
                "grilling",
            ),
            SkillStorageError::FormalDirectoryExists {
                name: "grilling".to_string(),
            }
        );
        assert_eq!(
            map_promote_error(io::Error::other("disk full"), "grilling"),
            SkillStorageError::OperationFailed {
                message: "disk full".to_string(),
            }
        );
    }

    #[test]
    fn keeps_journal_when_failed_rename_reached_its_destination() {
        let temp = TempDir::new().unwrap();
        let journal = temp.path().join("txn.json");
        let source = temp.path().join("staging");
        let promoted = temp.path().join("grilling");
        fs::write(&journal, "{}").unwrap();
        fs::create_dir_all(&promoted).unwrap();

        super::abandon_journal_after_rename_failure(&journal, &source, &promoted);

        assert!(journal.is_file());
        assert!(!source.exists());
        assert!(promoted.is_dir());
    }

    #[test]
    fn drops_journal_when_destination_occupancy_prevented_the_rename() {
        let temp = TempDir::new().unwrap();
        let journal = temp.path().join("txn.json");
        let source = temp.path().join("staging");
        let promoted = temp.path().join("grilling");
        fs::write(&journal, "{}").unwrap();
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&promoted).unwrap();

        super::abandon_journal_after_rename_failure(&journal, &source, &promoted);

        assert!(!journal.exists());
        assert!(source.is_dir());
        assert!(promoted.is_dir());
    }

    #[test]
    fn drops_journal_when_failed_rename_left_no_destination() {
        let temp = TempDir::new().unwrap();
        let journal = temp.path().join("txn.json");
        let source = temp.path().join("staging");
        fs::write(&journal, "{}").unwrap();
        fs::create_dir_all(&source).unwrap();

        super::abandon_journal_after_rename_failure(
            &journal,
            &source,
            &temp.path().join("missing"),
        );

        assert!(!journal.exists());
    }

    #[test]
    fn restores_backup_and_drops_journal_when_original_is_missing() {
        let temp = TempDir::new().unwrap();
        let original = temp.path().join("grilling");
        let backup = temp.path().join("backup");
        let journal = temp.path().join("txn.json");
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("SKILL.md"), "body").unwrap();
        fs::write(&journal, "{}").unwrap();

        super::restore_backup_or_keep_journal(&backup, &original, &journal);

        assert!(original.join("SKILL.md").is_file());
        assert!(!backup.exists());
        assert!(!journal.exists());
    }

    #[test]
    fn keeps_journal_when_original_exists_and_backup_remains() {
        let temp = TempDir::new().unwrap();
        let original = temp.path().join("grilling");
        let backup = temp.path().join("backup");
        let journal = temp.path().join("txn.json");
        fs::create_dir_all(&original).unwrap();
        fs::write(original.join("SKILL.md"), "new").unwrap();
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("SKILL.md"), "old").unwrap();
        fs::write(&journal, "{}").unwrap();

        super::restore_backup_or_keep_journal(&backup, &original, &journal);

        assert_eq!(
            fs::read_to_string(original.join("SKILL.md")).unwrap(),
            "new"
        );
        assert!(backup.join("SKILL.md").is_file());
        assert!(journal.is_file());
    }

    #[test]
    fn create_restores_staging_when_the_swapped_phase_cannot_be_written() {
        let temp = TempDir::new().unwrap();
        let storage = storage(&temp);
        let staging = storage.create_staging().unwrap();
        storage.write_manifest(&staging, b"new").unwrap();
        storage.fail_next_journal_phase_update();

        let error = storage
            .commit_create("grilling", &SkillId::new("skill-1"), &staging)
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "skill storage operation failed: injected journal phase update failure"
        );
        assert!(staging.join("SKILL.md").is_file());
        assert!(!temp.path().join("skills/grilling").exists());
        assert_eq!(storage.list_journals().unwrap(), Vec::new());
    }

    #[test]
    fn swap_restores_the_original_when_the_swapped_phase_cannot_be_written() {
        let temp = TempDir::new().unwrap();
        let storage = storage(&temp);
        let formal = temp.path().join("skills/grilling");
        fs::create_dir(&formal).unwrap();
        fs::write(formal.join("SKILL.md"), "old").unwrap();
        let staging = storage.create_staging().unwrap();
        storage.write_manifest(&staging, b"new").unwrap();
        storage.fail_next_journal_phase_update();
        let previous_updated_at = Some(100);

        storage
            .commit_swap(
                "grilling",
                "grilling",
                &SkillId::new("skill-1"),
                previous_updated_at,
                &staging,
            )
            .unwrap_err();

        assert_eq!(fs::read_to_string(formal.join("SKILL.md")).unwrap(), "old");
        assert!(!staging.exists());
        assert_eq!(storage.list_journals().unwrap(), Vec::new());
    }

    #[test]
    fn delete_restores_the_original_when_the_swapped_phase_cannot_be_written() {
        let temp = TempDir::new().unwrap();
        let storage = storage(&temp);
        let formal = temp.path().join("skills/grilling");
        fs::create_dir(&formal).unwrap();
        fs::write(formal.join("SKILL.md"), "old").unwrap();
        storage.fail_next_journal_phase_update();

        storage
            .commit_delete("grilling", &SkillId::new("skill-1"))
            .unwrap_err();

        assert_eq!(fs::read_to_string(formal.join("SKILL.md")).unwrap(), "old");
        assert_eq!(storage.list_journals().unwrap(), Vec::new());
        assert!(
            !temp
                .path()
                .join("skills")
                .join(JOURNAL_DIR_NAME)
                .read_dir()
                .unwrap()
                .any(|entry| entry.is_ok())
        );
    }
}
