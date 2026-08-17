use super::storage::{
    BACKUP_DIR_NAME, CreateHandle, DeleteHandle, JOURNAL_DIR_NAME, JournalOp, JournalPhase,
    STAGING_DIR_NAME, SkillStorage, SkillStorageError, SwapHandle, TransactionJournal,
};
use ora_utils::path::StrictRelativePath;
use std::fs;
use std::path::{Path, PathBuf};

/// Default filesystem implementation of [`SkillStorage`] rooted at the formal skills tree.
///
/// All transaction artifacts live under `<skills_root>/<reserved>/` so renames stay on the
/// same filesystem and startup recovery can deterministically resolve interrupted mutations.
#[derive(Debug, Clone)]
pub struct FilesystemSkillStorage {
    skills_root: PathBuf,
}

impl FilesystemSkillStorage {
    /// Builds storage rooted at the formal skill directory parent.
    pub fn new(skills_root: PathBuf) -> Self {
        Self { skills_root }
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
    /// Used by the workflow engine to materialize skills into a run worktree's `.claude/skills/`
    /// directory. `destination` is created when missing.
    pub fn copy_package_to(&self, name: &str, destination: &Path) -> Result<(), SkillStorageError> {
        let source = self.formal_path(name);
        if !source.is_dir() {
            return Err(SkillStorageError::FormalDirectoryMissing {
                name: name.to_string(),
            });
        }
        fs::create_dir_all(destination).map_err(map_storage_error)?;
        copy_dir_contents(&source, destination).map_err(|source| {
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
        fs::write(&journal.file, payload).map_err(map_storage_error)
    }

    /// Builds a journal marker for one transaction with deterministic paths.
    fn new_journal(
        &self,
        op: JournalOp,
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
        copy_dir_contents(&source, staging).map_err(map_storage_error)
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
        fs::write(&destination, bytes).map_err(map_storage_error)
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
        let mut input = fs::File::open(source).map_err(map_storage_error)?;
        let mut output = fs::File::create(&destination).map_err(map_storage_error)?;
        std::io::copy(&mut input, &mut output).map_err(map_storage_error)?;
        Ok(())
    }

    fn write_manifest(&self, staging: &Path, content: &[u8]) -> Result<(), SkillStorageError> {
        fs::write(staging.join("SKILL.md"), content).map_err(map_storage_error)
    }

    fn commit_create(&self, name: &str, staging: &Path) -> Result<CreateHandle, SkillStorageError> {
        let formal = self.formal_path(name);
        if formal.exists() {
            return Err(SkillStorageError::FormalDirectoryExists {
                name: name.to_string(),
            });
        }
        let mut journal = self.new_journal(JournalOp::Create, name, name, Some(staging), None);
        self.write_journal(&journal)?;
        if let Err(error) = fs::rename(staging, &formal) {
            let _ = fs::remove_file(&journal.file);
            return Err(map_storage_error(error));
        }
        self.update_journal_phase(&mut journal, JournalPhase::Swapped)?;
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
            JournalOp::Swap,
            name,
            from_name,
            Some(staging),
            Some(&backup),
        );
        self.write_journal(&journal)?;
        if let Some(parent) = backup.parent() {
            fs::create_dir_all(parent).map_err(map_storage_error)?;
        }

        if let Err(error) = fs::rename(&from_formal, &backup) {
            let _ = fs::remove_file(&journal.file);
            return Err(map_storage_error(error));
        }
        if let Err(error) = fs::rename(staging, &target_formal) {
            let _ = fs::rename(&backup, &from_formal);
            let _ = fs::remove_file(&journal.file);
            return Err(map_storage_error(error));
        }
        self.update_journal_phase(&mut journal, JournalPhase::Swapped)?;
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

    fn commit_delete(&self, name: &str) -> Result<DeleteHandle, SkillStorageError> {
        let formal = self.formal_path(name);
        if !formal.exists() {
            return Err(SkillStorageError::FormalDirectoryMissing {
                name: name.to_string(),
            });
        }
        let backup = self
            .reserved_root(BACKUP_DIR_NAME)
            .join(new_transaction_id());
        let mut journal = self.new_journal(JournalOp::Delete, name, name, None, Some(&backup));
        self.write_journal(&journal)?;
        if let Some(parent) = backup.parent() {
            fs::create_dir_all(parent).map_err(map_storage_error)?;
        }
        if let Err(error) = fs::rename(&formal, &backup) {
            let _ = fs::remove_file(&journal.file);
            return Err(map_storage_error(error));
        }
        self.update_journal_phase(&mut journal, JournalPhase::Swapped)?;
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
        fs::rename(backup, self.formal_path(name)).map_err(map_storage_error)
    }

    fn remove_dir(&self, path: &Path) -> Result<(), SkillStorageError> {
        fs::remove_dir_all(path).map_err(map_storage_error)
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
fn copy_dir_contents(source: &Path, destination: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            fs::create_dir_all(&destination_path)?;
            copy_dir_contents(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            let mut input = fs::File::open(&source_path)?;
            let mut output = fs::File::create(&destination_path)?;
            std::io::copy(&mut input, &mut output)?;
        }
    }
    Ok(())
}

/// Converts filesystem failures into stable storage-port errors.
fn map_storage_error(error: std::io::Error) -> SkillStorageError {
    SkillStorageError::OperationFailed {
        message: error.to_string(),
    }
}
