use ora_domain::{Skill, SkillId};
use std::path::Path;

/// Defines persistence operations required by the skill CRUD use cases.
pub trait SkillRepository {
    /// Persists a new skill snapshot.
    fn create_skill(&self, skill: Skill) -> Result<Skill, SkillRepositoryError>;

    /// Loads one visible skill by identifier.
    fn find_skill(&self, skill_id: &SkillId) -> Result<Option<Skill>, SkillRepositoryError>;

    /// Lists visible skills in deterministic storage order.
    fn list_skills(&self) -> Result<Vec<Skill>, SkillRepositoryError>;

    /// Replaces a visible skill identified by its stable identifier.
    fn update_skill(&self, skill: Skill) -> Result<Skill, SkillRepositoryError>;

    /// Marks a visible skill deleted at the supplied timestamp.
    fn soft_delete_skill(
        &self,
        skill_id: &SkillId,
        deleted_at: i64,
    ) -> Result<bool, SkillRepositoryError>;
}

/// Supplies new skill identifiers for create use cases.
pub trait SkillIdGenerator {
    /// Produces the identifier for a newly created skill.
    fn generate_skill_id(&self) -> SkillId;
}

/// Represents storage failures exposed as stable application outcomes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillRepositoryError {
    OperationFailed(String),
}

/// Owns the `atoms/skills` on-disk layout so import orchestration stays testable and hexagonal.
///
/// A skill is uploaded into an `<id>.tmp` staging directory keyed by its future identifier, then
/// promoted to a committed `<name>` directory once its manifest is parsed. Implementations must
/// keep those two directory shapes distinct because startup reconciliation relies on the `.tmp`
/// suffix to identify never-committed staging left behind by a crash.
pub trait SkillPackageStore {
    /// Creates the empty `<id>.tmp` staging directory for a new import.
    fn create_staging(&self, skill_id: &SkillId) -> Result<(), SkillPackageStoreError>;

    /// Writes one uploaded file at `relative_path` inside the staging directory, creating parents.
    fn write_file(
        &self,
        skill_id: &SkillId,
        relative_path: &Path,
        bytes: &[u8],
    ) -> Result<(), SkillPackageStoreError>;

    /// Reads the staged `SKILL.md` manifest contents, or `None` when the root has no manifest.
    fn read_manifest(&self, skill_id: &SkillId) -> Result<Option<String>, SkillPackageStoreError>;

    /// Reports whether a committed `<name>` directory already exists for a resolved skill name.
    fn committed_exists(&self, name: &str) -> Result<bool, SkillPackageStoreError>;

    /// Atomically renames the `<id>.tmp` staging directory to the committed `<name>` directory.
    ///
    /// Implementations must fail rather than overwrite when `<name>` already exists so a losing
    /// concurrent import cannot clobber a committed skill.
    fn promote(&self, skill_id: &SkillId, name: &str) -> Result<(), SkillPackageStoreError>;

    /// Removes the `<id>.tmp` staging directory when an import is rolled back.
    fn discard_staging(&self, skill_id: &SkillId) -> Result<(), SkillPackageStoreError>;

    /// Removes the committed `<name>` directory; used by delete, commit compensation, and GC.
    fn remove_committed(&self, name: &str) -> Result<(), SkillPackageStoreError>;

    /// Removes every `*.tmp` staging directory, discarding crash-orphaned incomplete imports.
    fn remove_all_staging(&self) -> Result<(), SkillPackageStoreError>;

    /// Lists the names of committed (non-`.tmp`) skill directories for reconciliation.
    fn list_committed_names(&self) -> Result<Vec<String>, SkillPackageStoreError>;
}

/// Represents skill filesystem failures exposed as stable application outcomes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillPackageStoreError {
    OperationFailed(String),
}

/// Runs the durable commit step of a skill import as one SQLite transaction.
///
/// Implementations insert the skill row, invoke `on_inserted` while the transaction is still open,
/// and commit only when it returns `Ok`. The `on_inserted` callback performs the filesystem
/// promote (rename), so the database row and the on-disk directory are made durable together: any
/// callback error rolls the row back, keeping "insert then rename then commit" atomic.
pub trait SkillImportUnitOfWork {
    /// Inserts `skill`, runs `on_inserted` inside the open transaction, and commits iff it succeeds.
    fn insert_then<OnInserted>(
        &self,
        skill: Skill,
        on_inserted: OnInserted,
    ) -> Result<(), SkillImportCommitError>
    where
        OnInserted: FnOnce() -> Result<(), String>;
}

/// Distinguishes the failure points of the import commit so the caller can compensate correctly.
///
/// The variants tell the orchestrator which side effects already happened: whether the on-disk
/// promote ran, and therefore whether it must discard the staging directory or remove a directory
/// that was already promoted before the commit failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillImportCommitError {
    /// The row insert failed before the promote ran; only staging needs discarding.
    Insert { message: String },
    /// The promote failed and the row was rolled back; staging still needs discarding.
    Promote { message: String },
    /// The promote succeeded but the commit failed; the promoted directory must be removed.
    CommitAfterPromote { message: String },
}
