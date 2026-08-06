use super::ports::{
    CandidateDecision, CandidateResultStatus, ConflictSkillInfo, ImportCandidate, ImportResult,
    ImportSessionState, SkillImportProgressEvent,
};
use crate::skill::SkillStorage;
use crate::{Clock, SkillRepository};
use ora_domain::{AuditFields, Skill, SkillId};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Internal snapshot handle materialized from the session's source tree.
pub(crate) struct SnapshotHandle {
    root: std::path::PathBuf,
}

/// The outcome of processing one candidate during the background commit.
#[derive(Debug, Clone)]
pub(crate) enum CandidateOutcome {
    Imported,
    Overwritten,
    Skipped,
    StaleConflict,
    Failed { error_code: String },
}

impl CandidateOutcome {
    /// Converts the outcome into its stored result projection.
    fn into_result(self, candidate: &ImportCandidate) -> ImportResult {
        let (status, error_code) = match self {
            CandidateOutcome::Imported => (CandidateResultStatus::Imported, None),
            CandidateOutcome::Overwritten => (CandidateResultStatus::Overwritten, None),
            CandidateOutcome::Skipped => (CandidateResultStatus::Skipped, None),
            CandidateOutcome::StaleConflict => (CandidateResultStatus::StaleConflict, None),
            CandidateOutcome::Failed { error_code } => {
                (CandidateResultStatus::Failed, Some(error_code))
            }
        };
        ImportResult {
            candidate_id: candidate.candidate_id.clone(),
            name: candidate.name.clone(),
            status,
            error_code,
        }
    }
}

/// Runs the whole commit in a detached thread: sequential candidates, per-item continuation.
///
/// The session state is mutated under its mutex so GET requests observe progress live, and the
/// temporary source snapshot is removed only after the last candidate finishes.
pub(crate) fn run_commit<Repository, Storage, IdGenerator, ClockSource, Progress>(
    repository: Repository,
    storage: Storage,
    id_generator: IdGenerator,
    clock: ClockSource,
    progress_publisher: Progress,
    session: Arc<Mutex<ImportSessionState>>,
) where
    Repository: SkillRepository + Send + Sync + 'static,
    Storage: SkillStorage + Send + Sync + 'static,
    IdGenerator: super::ports::SkillImportIdGenerator + Send + Sync + 'static,
    ClockSource: Clock + Send + Sync + 'static,
    Progress: super::ports::SkillImportProgressPublisher + Send + Sync + 'static,
{
    let locked = match session.lock() {
        Ok(state) => state,
        Err(_) => return,
    };
    let candidates = locked.candidates.clone();
    let decisions = locked.frozen_decisions.clone().unwrap_or_default();
    let session_root = locked.session_root.clone();
    drop(locked);

    let decisions: HashMap<String, CandidateDecision> = decisions.into_iter().collect();
    let snapshot = SnapshotHandle {
        root: session_root.join("snapshot"),
    };
    let mut ordered = candidates;
    ordered.sort_by(|left, right| left.source_path.cmp(&right.source_path));

    for candidate in &ordered {
        let outcome = commit_candidate(
            &repository,
            &storage,
            &id_generator,
            &clock,
            &snapshot,
            candidate,
            &decisions,
        );
        let result = outcome.into_result(candidate);
        if let Ok(mut state) = session.lock() {
            state.processed += 1;
            state.results.push(result);
            progress_publisher.publish(SkillImportProgressEvent {
                session_id: state.id.clone(),
                total: state.total,
                processed: state.processed,
            });
        }
    }

    if let Ok(mut state) = session.lock() {
        state.status = ora_contracts::SkillImportSessionStatus::Completed;
        state.terminal_at = Some(clock.now_timestamp_millis());
    }
    let _ = std::fs::remove_dir_all(&session_root);
}

/// Commits one candidate under its frozen decision, never aborting sibling candidates.
fn commit_candidate<Repository, Storage, IdGenerator, ClockSource>(
    repository: &Repository,
    storage: &Storage,
    id_generator: &IdGenerator,
    clock: &ClockSource,
    snapshot: &SnapshotHandle,
    candidate: &ImportCandidate,
    decisions: &HashMap<String, CandidateDecision>,
) -> CandidateOutcome
where
    Repository: SkillRepository,
    Storage: SkillStorage,
    IdGenerator: super::ports::SkillImportIdGenerator,
    ClockSource: Clock,
{
    match candidate.status {
        super::ports::CandidateStatus::Invalid => CandidateOutcome::Failed {
            error_code: "invalid_candidate".to_string(),
        },
        super::ports::CandidateStatus::Ready => import_new(
            repository,
            storage,
            id_generator,
            clock,
            snapshot,
            candidate,
        ),
        super::ports::CandidateStatus::Conflict => match decisions.get(&candidate.candidate_id) {
            Some(CandidateDecision::Skip) => CandidateOutcome::Skipped,
            Some(CandidateDecision::Overwrite) => overwrite_existing(
                repository,
                storage,
                id_generator,
                clock,
                snapshot,
                candidate,
            ),
            None => CandidateOutcome::Failed {
                error_code: "decision_missing".to_string(),
            },
        },
    }
}

/// Creates one new skill, staging its boundary and promoting it atomically with the DB row.
fn import_new<Repository, Storage, IdGenerator, ClockSource>(
    repository: &Repository,
    storage: &Storage,
    id_generator: &IdGenerator,
    clock: &ClockSource,
    snapshot: &SnapshotHandle,
    candidate: &ImportCandidate,
) -> CandidateOutcome
where
    Repository: SkillRepository,
    Storage: SkillStorage,
    IdGenerator: super::ports::SkillImportIdGenerator,
    ClockSource: Clock,
{
    // A `ready` name must still be free at commit time (optimistic concurrency).
    match repository.find_skill_by_name(&candidate.name) {
        Ok(Some(_)) => return CandidateOutcome::StaleConflict,
        Err(_) => {
            return CandidateOutcome::Failed {
                error_code: "skill_repository_error".to_string(),
            };
        }
        Ok(None) => {}
    }

    let now = clock.now_timestamp_millis();
    let skill = match Skill::new(
        SkillId::new(id_generator.generate_import_id()),
        candidate.name.clone(),
        candidate.description.clone(),
        AuditFields::new(now, now, false),
    ) {
        Ok(skill) => skill,
        Err(_) => {
            return CandidateOutcome::Failed {
                error_code: "skill_name_invalid".to_string(),
            };
        }
    };

    let staging = match storage.create_staging() {
        Ok(staging) => staging,
        Err(_) => {
            return CandidateOutcome::Failed {
                error_code: "skill_storage_error".to_string(),
            };
        }
    };
    if let Err(error_code) = stage_boundary(storage, &staging, snapshot, candidate) {
        return CandidateOutcome::Failed { error_code };
    }
    let handle = match storage.commit_create(&skill.name, &staging) {
        Ok(handle) => handle,
        Err(_) => {
            return CandidateOutcome::Failed {
                error_code: "skill_storage_error".to_string(),
            };
        }
    };
    if repository.create_skill(skill).is_err() {
        let _ = storage.rollback_create(&handle);
        return CandidateOutcome::Failed {
            error_code: "skill_repository_error".to_string(),
        };
    }
    let _ = storage.finish_create(&handle);
    CandidateOutcome::Imported
}

/// Overwrites an existing skill, revalidating the frozen id/updatedAt before writing.
#[allow(clippy::too_many_arguments)]
fn overwrite_existing<Repository, Storage, IdGenerator, ClockSource>(
    repository: &Repository,
    storage: &Storage,
    _id_generator: &IdGenerator,
    clock: &ClockSource,
    snapshot: &SnapshotHandle,
    candidate: &ImportCandidate,
) -> CandidateOutcome
where
    Repository: SkillRepository,
    Storage: SkillStorage,
    IdGenerator: super::ports::SkillImportIdGenerator,
    ClockSource: Clock,
{
    let Some(existing) = &candidate.existing_skill else {
        return CandidateOutcome::Failed {
            error_code: "invalid_candidate".to_string(),
        };
    };
    let Some(frozen) = frozen_conflict(existing, repository, candidate) else {
        return CandidateOutcome::StaleConflict;
    };

    let now = clock.now_timestamp_millis();
    let skill = match Skill::new(
        frozen.id.clone(),
        candidate.name.clone(),
        candidate.description.clone(),
        AuditFields::new(frozen.created_at, now, false),
    ) {
        Ok(skill) => skill,
        Err(_) => {
            return CandidateOutcome::Failed {
                error_code: "skill_name_invalid".to_string(),
            };
        }
    };

    let staging = match storage.create_staging() {
        Ok(staging) => staging,
        Err(_) => {
            return CandidateOutcome::Failed {
                error_code: "skill_storage_error".to_string(),
            };
        }
    };
    if let Err(error_code) = stage_boundary(storage, &staging, snapshot, candidate) {
        return CandidateOutcome::Failed { error_code };
    }
    let handle = match storage.commit_swap(&candidate.name, &existing.name, &staging) {
        Ok(handle) => handle,
        Err(_) => {
            return CandidateOutcome::Failed {
                error_code: "skill_storage_error".to_string(),
            };
        }
    };
    if repository.update_skill(skill).is_err() {
        let _ = storage.rollback_swap(&handle);
        return CandidateOutcome::Failed {
            error_code: "skill_repository_error".to_string(),
        };
    }
    let _ = storage.finish_swap(&handle);
    CandidateOutcome::Overwritten
}

/// Revalidates the frozen conflict target against the live database at commit time.
fn frozen_conflict<Repository>(
    existing: &ConflictSkillInfo,
    repository: &Repository,
    candidate: &ImportCandidate,
) -> Option<SkillSnapshot>
where
    Repository: SkillRepository,
{
    let current = match repository.find_skill(&existing.skill_id) {
        Ok(Some(current)) => current,
        _ => return None,
    };
    if current.audit_fields.updated_at != existing.updated_at {
        return None;
    }
    // The name must not have been claimed by a different visible skill since preview.
    if let Ok(Some(other)) = repository.find_skill_by_name(&candidate.name)
        && other.id != existing.skill_id
    {
        return None;
    }
    Some(SkillSnapshot {
        id: current.id,
        created_at: current.audit_fields.created_at,
    })
}

/// The revalidated identity snapshot needed to overwrite one skill.
struct SkillSnapshot {
    id: SkillId,
    created_at: i64,
}

/// Copies every boundary file from the snapshot into a staging directory.
fn stage_boundary<Storage: SkillStorage>(
    storage: &Storage,
    staging: &Path,
    snapshot: &SnapshotHandle,
    candidate: &ImportCandidate,
) -> Result<(), String> {
    let Some(root) = candidate.boundary.manifest_path.parent() else {
        return Err("skill_storage_error".to_string());
    };
    for file in &candidate.boundary.files {
        let within = file
            .relative_path
            .strip_prefix(&root)
            .ok_or_else(|| "skill_storage_error".to_string())?;
        let source = file.relative_path.to_path(&snapshot.root);
        storage
            .copy_file(staging, &within, &source)
            .map_err(|_| "skill_storage_error".to_string())?;
    }
    Ok(())
}
