use ora_domain::SkillId;
use ora_skill_package::{Limits, SkillBoundary};
use ora_utils::path::StrictRelativePath;
use std::path::PathBuf;
use std::time::Duration;

/// Names the runtime tunables that govern one import session.
#[derive(Debug, Clone)]
pub struct SkillImportConfig {
    /// Root directory for session snapshots in OS temporary storage.
    pub temp_root: PathBuf,
    /// Resource limits applied while materializing the source snapshot.
    pub limits: Limits,
    /// Idle timeout after which a prepared session is expired on access.
    pub idle_timeout: Duration,
    /// Absolute maximum lifetime of a prepared session.
    pub max_lifetime: Duration,
    /// How long completed/cancelled result metadata is retained.
    pub result_retention: Duration,
    /// Wall-clock budget for source materialization, scanning, and manifest parsing.
    pub preparation_timeout: Duration,
}

impl Default for SkillImportConfig {
    /// Selects the production defaults: OS temp, standard limits, and spec durations.
    fn default() -> Self {
        Self {
            temp_root: std::env::temp_dir(),
            limits: Limits::default(),
            idle_timeout: Duration::from_secs(30 * 60),
            max_lifetime: Duration::from_secs(2 * 60 * 60),
            result_retention: Duration::from_secs(30 * 60),
            preparation_timeout: Duration::from_secs(120),
        }
    }
}

/// Supplies unpredictable identifiers for import sessions and candidates.
pub trait SkillImportIdGenerator {
    /// Produces one new opaque identifier that is never used as a filesystem path directly.
    fn generate_import_id(&self) -> String;
}

/// Generates opaque UUID identifiers for import sessions and candidates.
#[derive(Debug, Clone, Copy, Default)]
pub struct UuidSkillImportIdGenerator;

impl SkillImportIdGenerator for UuidSkillImportIdGenerator {
    /// Produces one new UUIDv4 identifier.
    fn generate_import_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}

/// Publishes lightweight commit-progress events for the message bus adapter.
///
/// The application layer only defines the port; transport adapters wire a real publisher.
/// Large result objects are never sent through this channel — subscribers poll the session
/// endpoint for the authoritative state.
pub trait SkillImportProgressPublisher {
    /// Emits one progress snapshot for the named session.
    fn publish(&self, event: SkillImportProgressEvent);
}

/// One lightweight progress event emitted during a background commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillImportProgressEvent {
    pub session_id: String,
    pub total: usize,
    pub processed: usize,
}

/// Default publisher that discards events until a real message bus is wired.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopSkillImportProgressPublisher;

impl SkillImportProgressPublisher for NoopSkillImportProgressPublisher {
    fn publish(&self, _event: SkillImportProgressEvent) {}
}

/// The prepared status of one previewed candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateStatus {
    Ready,
    Conflict,
    Invalid,
}

/// The frozen user decision applied to one conflict candidate at commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateDecision {
    Skip,
    Overwrite,
}

/// The per-candidate outcome produced by the background commit task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateResultStatus {
    Imported,
    Overwritten,
    Skipped,
    Failed,
    StaleConflict,
}

/// One previewed skill candidate with its ownership boundary and existing-skill state.
#[derive(Debug, Clone)]
pub struct ImportCandidate {
    pub candidate_id: String,
    pub name: String,
    pub description: String,
    /// Validated source-relative path of the candidate's `SKILL.md`.
    pub source_path: StrictRelativePath,
    /// The non-overlapping boundary this candidate owns.
    pub boundary: SkillBoundary,
    pub status: CandidateStatus,
    /// Stable machine-readable reason for `Invalid` candidates.
    pub error_code: Option<String>,
    /// Present only for `Conflict` candidates.
    pub existing_skill: Option<ConflictSkillInfo>,
}

/// The visible skill a conflict candidate would overwrite, for commit-time revalidation.
#[derive(Debug, Clone)]
pub struct ConflictSkillInfo {
    pub skill_id: SkillId,
    /// The persisted name at preview time, used as the swap source during overwrite.
    pub name: String,
    pub updated_at: i64,
    pub description: String,
}

/// One finished per-candidate result.
#[derive(Debug, Clone)]
pub struct ImportResult {
    pub candidate_id: String,
    pub name: String,
    pub status: CandidateResultStatus,
    pub error_code: Option<String>,
}

/// The in-memory state of one import session shared between requests and the commit task.
#[derive(Debug)]
pub struct ImportSessionState {
    pub id: String,
    pub status: ora_contracts::SkillImportSessionStatus,
    pub created_at: i64,
    pub last_accessed_at: i64,
    pub terminal_at: Option<i64>,
    /// Absolute session directory in OS temporary storage (cleaned after terminal states).
    pub session_root: PathBuf,
    pub candidates: Vec<ImportCandidate>,
    /// Frozen commit decisions, present once a commit has been accepted.
    pub frozen_decisions: Option<Vec<(String, CandidateDecision)>>,
    pub total: usize,
    pub processed: usize,
    pub results: Vec<ImportResult>,
}
