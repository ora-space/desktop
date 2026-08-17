use ora_utils::path::StrictRelativePath;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Names of the directories this layer reserves under the skills root.
///
/// They are owned by `ora-domain` because [`ora_domain::validate_skill_name`] rejects every
/// dot-prefixed skill name, which keeps these dot-prefixed directories unclaimable without
/// needing to enumerate them individually. Re-exporting instead of redeclaring keeps one literal
/// per directory, so the two layers cannot drift apart.
pub use ora_domain::{BACKUP_DIR_NAME, JOURNAL_DIR_NAME, STAGING_DIR_NAME};

/// Captures one durable intent record written before a formal skill mutation.
///
/// The journal lets startup recovery restore or clean a transaction that was interrupted
/// between the filesystem swap and the database write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionJournal {
    pub op: JournalOp,
    /// Target (new) skill name the transaction writes.
    pub name: String,
    /// Previous skill name before the transaction (equal to `name` for create/delete).
    pub from_name: String,
    /// Absolute staging directory path.
    pub staging: String,
    /// Absolute compensation backup directory path (empty when not yet created).
    pub backup: String,
    pub phase: JournalPhase,
    /// Absolute path of the journal marker file itself.
    pub file: String,
}

/// Kinds of durable skill mutation that participate in startup recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JournalOp {
    Create,
    Swap,
    Delete,
}

/// Tracks whether the filesystem swap completed before the database write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JournalPhase {
    Prepared,
    Swapped,
}

/// Handle for a pending create transaction returned by [`SkillStorage::commit_create`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateHandle {
    pub name: String,
    pub staging: PathBuf,
    pub journal: PathBuf,
}

/// Handle for a pending overwrite/rename transaction returned by [`SkillStorage::commit_swap`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwapHandle {
    pub name: String,
    pub from_name: String,
    pub staging: PathBuf,
    pub backup: PathBuf,
    pub journal: PathBuf,
}

/// Handle for a pending delete transaction returned by [`SkillStorage::commit_delete`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteHandle {
    pub name: String,
    pub backup: PathBuf,
    pub journal: PathBuf,
}

/// Reports filesystem failures surfaced by the skill storage port.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SkillStorageError {
    #[error("skill formal directory is missing: {name}")]
    FormalDirectoryMissing { name: String },
    #[error("skill formal directory already exists: {name}")]
    FormalDirectoryExists { name: String },
    #[error("skill storage operation failed: {message}")]
    OperationFailed { message: String },
}

/// Supplies the on-disk formal skill tree operations required by atomic CRUD and import.
///
/// Implementations must keep formal directories at `<skills_root>/<name>/` with a root
/// `SKILL.md`, reserve transaction staging under `<skills_root>/<STAGING_DIR_NAME>`, keep
/// compensation backups under `<skills_root>/<BACKUP_DIR_NAME>`, and record intent in
/// `<skills_root>/<JOURNAL_DIR_NAME>`. All staging, backup, and journal paths live on the
/// same filesystem as the formal tree so promotion uses rename instead of cross-device copies.
///
/// Reserved directory names and committed skill names occupy disjoint namespaces:
/// [`ora_domain::validate_skill_name`] refuses every dot-prefixed name, which keeps any
/// dot-prefixed reserved directory unclaimable. Without that split a skill would be promoted
/// onto a transaction root and startup reconciliation would delete its package as a leftover, so
/// a new reserved directory must keep the leading dot rather than needing a validation change.
pub trait SkillStorage {
    /// Reserves a unique staging directory for one transaction.
    fn create_staging(&self) -> Result<PathBuf, SkillStorageError>;

    /// Copies the entire formal package for `name` into a staging directory.
    ///
    /// Used by update so an atomic swap preserves every package file the user did not modify
    /// while the handler rewrites only the manifest inside the staging copy.
    fn stage_existing(&self, name: &str, staging: &Path) -> Result<(), SkillStorageError>;

    /// Writes one ordinary file into a staging directory at a validated relative path.
    fn write_file(
        &self,
        staging: &Path,
        relative: &StrictRelativePath,
        bytes: &[u8],
    ) -> Result<(), SkillStorageError>;

    /// Streams one source file into a staging directory at a validated relative path.
    ///
    /// Import commit uses this so package bytes are copied in bounded chunks instead of being
    /// buffered whole, and the destination is created with application-defined permissions.
    fn copy_file(
        &self,
        staging: &Path,
        relative: &StrictRelativePath,
        source: &Path,
    ) -> Result<(), SkillStorageError>;

    /// Writes the `SKILL.md` content into a staging directory root.
    fn write_manifest(&self, staging: &Path, content: &[u8]) -> Result<(), SkillStorageError>;

    /// Promotes a staging directory to the formal `<name>` directory for a new skill.
    fn commit_create(&self, name: &str, staging: &Path) -> Result<CreateHandle, SkillStorageError>;

    /// Removes a partially created formal directory and cleans the staging and journal.
    fn rollback_create(&self, handle: &CreateHandle) -> Result<(), SkillStorageError>;

    /// Finalizes a successful create by removing the journal and leftover staging.
    fn finish_create(&self, handle: &CreateHandle) -> Result<(), SkillStorageError>;

    /// Swaps the formal directory for `from_name` (or `name`) with staging content.
    fn commit_swap(
        &self,
        name: &str,
        from_name: &str,
        staging: &Path,
    ) -> Result<SwapHandle, SkillStorageError>;

    /// Restores the original formal directory after an interrupted swap.
    fn rollback_swap(&self, handle: &SwapHandle) -> Result<(), SkillStorageError>;

    /// Finalizes a successful swap by removing the journal and compensation backup.
    fn finish_swap(&self, handle: &SwapHandle) -> Result<(), SkillStorageError>;

    /// Moves the formal `<name>` directory into a compensation backup for deletion.
    fn commit_delete(&self, name: &str) -> Result<DeleteHandle, SkillStorageError>;

    /// Restores the formal directory after an interrupted delete.
    fn rollback_delete(&self, handle: &DeleteHandle) -> Result<(), SkillStorageError>;

    /// Finalizes a successful delete by removing the journal and backup directory.
    fn finish_delete(&self, handle: &DeleteHandle) -> Result<(), SkillStorageError>;

    /// Returns whether a formal directory exists for the name.
    fn formal_exists(&self, name: &str) -> bool;

    /// Reads the formal `SKILL.md` bytes, if the directory exists.
    fn read_manifest(&self, name: &str) -> Result<Option<Vec<u8>>, SkillStorageError>;

    /// Lists formal directory names excluding reserved transaction directories.
    fn list_formal_names(&self) -> Result<Vec<String>, SkillStorageError>;

    /// Removes one temporary staging or backup directory.
    fn remove_temp(&self, path: &Path) -> Result<(), SkillStorageError>;

    /// Restores a compensation backup directory to the formal `<name>` location.
    fn restore_backup(&self, backup: &Path, name: &str) -> Result<(), SkillStorageError>;

    /// Removes one directory (best effort by the caller's recovery policy).
    fn remove_dir(&self, path: &Path) -> Result<(), SkillStorageError>;

    /// Lists every unresolved transaction journal for startup recovery.
    fn list_journals(&self) -> Result<Vec<TransactionJournal>, SkillStorageError>;

    /// Removes one resolved journal file.
    fn remove_journal(&self, journal: &TransactionJournal) -> Result<(), SkillStorageError>;
}
