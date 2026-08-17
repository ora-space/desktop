use super::commit::run_commit;
use super::errors::{DuplicateSkillName, SkillImportError};
use super::mapper::{project_progress, project_session, to_internal_decision};
use super::ports::{
    CandidateDecision, CandidateStatus, ConflictSkillInfo, ImportCandidate, ImportSessionState,
    SkillImportConfig, SkillImportIdGenerator, SkillImportProgressPublisher,
};
use crate::skill::SkillStorage;
use crate::{ApplicationError, Clock, SkillRepository};
use ora_contracts::{
    CancelSkillImportRequest, CancelSkillImportResponse, CommitSkillImportRequest,
    CommitSkillImportResponse, GetSkillImportSessionRequest, GetSkillImportSessionResponse,
    PrepareSkillImportRequest, PrepareSkillImportResponse, SkillImportSource,
};
use ora_domain::Namespace;
use ora_skill_package::manifest::{ManifestError, parse_manifest};
use ora_skill_package::{
    ArchiveFormat, ExtractedTree, Limits, SkillBoundary, SkillSource, materialize_source,
    scan_skill_boundaries,
};
use ora_utils::path::StrictRelativePath;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Owns the in-memory import session lifecycle shared by Web and Desktop adapters.
pub struct SkillImportService<Repository, Storage, IdGenerator, ClockSource, Progress> {
    repository: Repository,
    storage: Storage,
    id_generator: IdGenerator,
    clock: ClockSource,
    progress_publisher: Progress,
    config: SkillImportConfig,
    sessions: Arc<Mutex<HashMap<String, Arc<Mutex<ImportSessionState>>>>>,
}

impl<Repository, Storage, IdGenerator, ClockSource, Progress>
    SkillImportService<Repository, Storage, IdGenerator, ClockSource, Progress>
{
    pub fn new(
        repository: Repository,
        storage: Storage,
        id_generator: IdGenerator,
        clock: ClockSource,
        progress_publisher: Progress,
        config: SkillImportConfig,
    ) -> Self {
        Self {
            repository,
            storage,
            id_generator,
            clock,
            progress_publisher,
            config,
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl<Repository, Storage, IdGenerator, ClockSource, Progress>
    SkillImportService<Repository, Storage, IdGenerator, ClockSource, Progress>
where
    Repository: SkillRepository + Clone + Send + Sync + 'static,
    Storage: SkillStorage + Clone + Send + Sync + 'static,
    IdGenerator: SkillImportIdGenerator + Clone + Send + Sync + 'static,
    ClockSource: Clock + Clone + Send + Sync + 'static,
    Progress: SkillImportProgressPublisher + Clone + Send + Sync + 'static,
{
    /// Prepares one source, returning a previewed session without touching formal storage.
    pub fn prepare(
        &self,
        request: PrepareSkillImportRequest,
    ) -> Result<PrepareSkillImportResponse, ApplicationError> {
        let session_id = self.id_generator.generate_import_id();
        let session_root = self
            .config
            .temp_root
            .join(format!("ora-skill-import-{session_id}"));
        fs::create_dir_all(&session_root).map_err(map_io_error)?;

        let repository = self.repository.clone();
        let id_generator = self.id_generator.clone();
        let limits = self.config.limits.clone();
        let preparation_timeout = self.config.preparation_timeout;
        let source = request.source;
        let thread_session_root = session_root.clone();

        let (sender, receiver) = std::sync::mpsc::channel();
        let _ = std::thread::spawn(move || {
            let result = prepare_candidates(
                &repository,
                &id_generator,
                &limits,
                &thread_session_root,
                &source,
            );
            let _ = sender.send(result);
        });
        let candidates = match receiver.recv_timeout(preparation_timeout) {
            Ok(Ok(candidates)) => candidates,
            Ok(Err(error)) => {
                let _ = fs::remove_dir_all(&session_root);
                return Err(ApplicationError::SkillImport(error));
            }
            Err(_) => {
                let _ = fs::remove_dir_all(&session_root);
                return Err(ApplicationError::SkillImport(
                    SkillImportError::PreparationTimeout,
                ));
            }
        };

        let now = self.clock.now_timestamp_millis();
        let total = candidates
            .iter()
            .filter(|candidate| candidate.status != CandidateStatus::Invalid)
            .count();
        let state = ImportSessionState {
            id: session_id.clone(),
            status: ora_contracts::SkillImportSessionStatus::Prepared,
            created_at: now,
            last_accessed_at: now,
            terminal_at: None,
            session_root,
            candidates,
            frozen_decisions: None,
            total,
            processed: 0,
            results: Vec::new(),
        };
        let session = Arc::new(Mutex::new(state));
        let projection = {
            let state = session.lock().map_err(lock_error)?;
            project_session(&state)
        };
        let mut sessions = self.sessions.lock().map_err(lock_error)?;
        sessions.insert(session_id, session);
        Ok(PrepareSkillImportResponse {
            session: projection,
        })
    }

    /// Returns one session projection, expiring stale sessions lazily on access.
    pub fn get_session(
        &self,
        request: GetSkillImportSessionRequest,
    ) -> Result<GetSkillImportSessionResponse, ApplicationError> {
        let now = self.clock.now_timestamp_millis();
        let session = self
            .sessions
            .lock()
            .map_err(lock_error)?
            .get(&request.session_id)
            .cloned()
            .ok_or_else(|| {
                ApplicationError::SkillImport(SkillImportError::SessionNotFound {
                    session_id: request.session_id.clone(),
                })
            })?;

        let (expired, status) = {
            let state = session.lock().map_err(lock_error)?;
            (self.is_expired(&state, now), state.status.clone())
        };
        if expired {
            self.remove_session(&request.session_id);
            return Err(ApplicationError::SkillImport(
                SkillImportError::SessionExpired,
            ));
        }
        if status == ora_contracts::SkillImportSessionStatus::Prepared
            && let Ok(mut state) = session.lock()
        {
            state.last_accessed_at = now;
        }

        let state = session.lock().map_err(lock_error)?;
        Ok(GetSkillImportSessionResponse {
            session: project_session(&state),
        })
    }

    /// Validates and freezes the commit decisions, then starts the background commit task.
    pub fn commit(
        &self,
        request: CommitSkillImportRequest,
    ) -> Result<CommitSkillImportResponse, ApplicationError> {
        let session = self
            .sessions
            .lock()
            .map_err(lock_error)?
            .get(&request.session_id)
            .cloned()
            .ok_or_else(|| {
                ApplicationError::SkillImport(SkillImportError::SessionNotFound {
                    session_id: request.session_id.clone(),
                })
            })?;

        let response: CommitSkillImportResponse = {
            let mut state = session.lock().map_err(lock_error)?;
            let now = self.clock.now_timestamp_millis();
            if state.status == ora_contracts::SkillImportSessionStatus::Prepared
                && self.is_expired(&state, now)
            {
                let session_id = state.id.clone();
                drop(state);
                self.remove_session(&session_id);
                return Err(ApplicationError::SkillImport(
                    SkillImportError::SessionExpired,
                ));
            }

            match state.status.clone() {
                ora_contracts::SkillImportSessionStatus::Cancelled => {
                    return Err(ApplicationError::SkillImport(
                        SkillImportError::SessionCancelled,
                    ));
                }
                ora_contracts::SkillImportSessionStatus::Committing => {
                    return Err(ApplicationError::SkillImport(
                        SkillImportError::CommitInProgress,
                    ));
                }
                ora_contracts::SkillImportSessionStatus::Completed => {
                    if self.decisions_match(&state, &request.decisions) {
                        return Ok(CommitSkillImportResponse {
                            session_id: state.id.clone(),
                            status: state.status.clone(),
                            progress: project_progress(&state),
                        });
                    }
                    return Err(ApplicationError::SkillImport(
                        SkillImportError::AlreadyCommitted,
                    ));
                }
                ora_contracts::SkillImportSessionStatus::Prepared => {
                    let decisions = self.validate_decisions(&state, &request.decisions)?;
                    state.frozen_decisions = Some(decisions);
                    state.status = ora_contracts::SkillImportSessionStatus::Committing;
                    state.processed = 0;
                    state.results.clear();
                    Ok::<CommitSkillImportResponse, ApplicationError>(CommitSkillImportResponse {
                        session_id: state.id.clone(),
                        status: state.status.clone(),
                        progress: project_progress(&state),
                    })
                }
            }
        }?;

        let repository = self.repository.clone();
        let storage = self.storage.clone();
        let id_generator = self.id_generator.clone();
        let clock = self.clock.clone();
        let progress_publisher = self.progress_publisher.clone();
        let _ = std::thread::spawn(move || {
            run_commit(
                repository,
                storage,
                id_generator,
                clock,
                progress_publisher,
                session,
            );
        });
        Ok(response)
    }

    /// Cancels a prepared session; a commit that started is never cancellable.
    pub fn cancel(
        &self,
        request: CancelSkillImportRequest,
    ) -> Result<CancelSkillImportResponse, ApplicationError> {
        let session = self
            .sessions
            .lock()
            .map_err(lock_error)?
            .get(&request.session_id)
            .cloned()
            .ok_or_else(|| {
                ApplicationError::SkillImport(SkillImportError::SessionNotFound {
                    session_id: request.session_id.clone(),
                })
            })?;

        let (cancelled, terminal) = {
            let mut state = session.lock().map_err(lock_error)?;
            let now = self.clock.now_timestamp_millis();
            if state.status == ora_contracts::SkillImportSessionStatus::Prepared
                && self.is_expired(&state, now)
            {
                let session_id = state.id.clone();
                drop(state);
                self.remove_session(&session_id);
                return Err(ApplicationError::SkillImport(
                    SkillImportError::SessionExpired,
                ));
            }
            match state.status {
                ora_contracts::SkillImportSessionStatus::Prepared => {
                    state.status = ora_contracts::SkillImportSessionStatus::Cancelled;
                    state.terminal_at = Some(now);
                    (true, Some(state.session_root.clone()))
                }
                ora_contracts::SkillImportSessionStatus::Committing => {
                    return Err(ApplicationError::SkillImport(
                        SkillImportError::CommitInProgress,
                    ));
                }
                _ => (false, None),
            }
        };
        if let Some(root) = terminal {
            let _ = fs::remove_dir_all(&root);
        }
        Ok(CancelSkillImportResponse {
            session_id: request.session_id,
            cancelled,
        })
    }

    /// Removes an expired session from the map and cleans its temporary snapshot.
    fn remove_session(&self, session_id: &str) {
        let removed = self
            .sessions
            .lock()
            .map(|mut sessions| sessions.remove(session_id))
            .ok()
            .flatten();
        if let Some(session) = removed
            && let Ok(state) = session.lock()
        {
            let _ = fs::remove_dir_all(&state.session_root);
        }
    }

    /// Returns whether a session has outlived its lifetime budget without completion.
    fn is_expired(&self, state: &ImportSessionState, now: i64) -> bool {
        match state.status {
            ora_contracts::SkillImportSessionStatus::Prepared => {
                let idle_exceeded =
                    now - state.last_accessed_at > duration_millis(self.config.idle_timeout);
                let lifetime_exceeded =
                    now - state.created_at > duration_millis(self.config.max_lifetime);
                idle_exceeded || lifetime_exceeded
            }
            // Commit keeps running regardless of idle time; only completion starts retention.
            ora_contracts::SkillImportSessionStatus::Committing => false,
            ora_contracts::SkillImportSessionStatus::Completed
            | ora_contracts::SkillImportSessionStatus::Cancelled => {
                state.terminal_at.is_some_and(|terminal_at| {
                    now - terminal_at > duration_millis(self.config.result_retention)
                })
            }
        }
    }

    /// Validates that every conflict candidate has an explicit decision.
    fn validate_decisions(
        &self,
        state: &ImportSessionState,
        decisions: &[ora_contracts::SkillImportConflictDecision],
    ) -> Result<Vec<(String, CandidateDecision)>, ApplicationError> {
        let provided: HashSet<&str> = decisions
            .iter()
            .map(|decision| decision.candidate_id.as_str())
            .collect();
        let missing = state
            .candidates
            .iter()
            .filter(|candidate| candidate.status == CandidateStatus::Conflict)
            .map(|candidate| candidate.candidate_id.as_str())
            .filter(|candidate_id| !provided.contains(candidate_id))
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(ApplicationError::SkillImport(
                SkillImportError::DecisionMissing {
                    candidate_ids: missing,
                },
            ));
        }

        let mut normalized = decisions
            .iter()
            .map(|decision| {
                (
                    decision.candidate_id.clone(),
                    to_internal_decision(&decision.decision),
                )
            })
            .collect::<Vec<_>>();
        normalized.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(normalized)
    }

    /// Compares an idempotent retry's decisions against the frozen commit decisions.
    fn decisions_match(
        &self,
        state: &ImportSessionState,
        decisions: &[ora_contracts::SkillImportConflictDecision],
    ) -> bool {
        let Some(frozen) = &state.frozen_decisions else {
            return false;
        };
        let mut normalized = decisions
            .iter()
            .map(|decision| {
                (
                    decision.candidate_id.clone(),
                    to_internal_decision(&decision.decision),
                )
            })
            .collect::<Vec<_>>();
        normalized.sort_by(|left, right| left.0.cmp(&right.0));
        *frozen == normalized
    }
}

/// Materializes and scans one source into prepared candidates (runs under the timeout budget).
fn prepare_candidates<Repository, IdGenerator>(
    repository: &Repository,
    id_generator: &IdGenerator,
    limits: &Limits,
    session_root: &Path,
    source: &SkillImportSource,
) -> Result<Vec<ImportCandidate>, SkillImportError>
where
    Repository: SkillRepository,
    IdGenerator: SkillImportIdGenerator,
{
    let snapshot_root = session_root.join("snapshot");
    let snapshot = match source {
        SkillImportSource::Folder { path } => materialize_source(
            SkillSource::Folder {
                path: Path::new(path),
            },
            &snapshot_root,
            limits,
        )?,
        SkillImportSource::Archive { path, file_name } => {
            let format = ArchiveFormat::from_extension(file_name)
                .ok_or(SkillImportError::ArchiveFormatUnsupported)?;
            let raw = session_root.join("source-upload");
            copy_archive_with_limit(Path::new(path), &raw, limits.extract.max_archive_bytes)?;
            materialize_source(
                SkillSource::Archive { path: &raw, format },
                &snapshot_root,
                limits,
            )?
        }
    };

    let boundaries = scan_skill_boundaries(&snapshot);
    if boundaries.is_empty() {
        return Err(SkillImportError::SkillManifestNotFound);
    }
    if boundaries.len() > limits.max_skills {
        return Err(SkillImportError::TooManySkills {
            max_skills: limits.max_skills,
        });
    }

    let mut candidates = Vec::with_capacity(boundaries.len());
    for boundary in &boundaries {
        if boundary.file_count() > limits.max_files_per_skill {
            return Err(SkillImportError::TooManyFiles {
                max_files: limits.max_files_per_skill,
            });
        }
        candidates.push(build_candidate(
            &snapshot,
            boundary,
            repository,
            id_generator,
            limits,
        )?);
    }
    reject_duplicate_names(&candidates)?;
    Ok(candidates)
}

/// Builds one candidate from its boundary, parsing the manifest and querying conflicts.
fn build_candidate<Repository, IdGenerator>(
    snapshot: &ExtractedTree,
    boundary: &SkillBoundary,
    repository: &Repository,
    id_generator: &IdGenerator,
    limits: &Limits,
) -> Result<ImportCandidate, SkillImportError>
where
    Repository: SkillRepository,
    IdGenerator: SkillImportIdGenerator,
{
    let candidate_id = id_generator.generate_import_id();
    let source_path = boundary.manifest_path.clone();
    let manifest_bytes =
        read_manifest_capped(snapshot, &boundary.manifest_path, limits.max_manifest_bytes)?;

    match parse_manifest(&manifest_bytes, limits.max_manifest_bytes) {
        Ok(manifest) => {
            let existing = repository
                .find_skill_by_name(&Namespace::local(), &manifest.name)
                .map_err(|error| SkillImportError::Repository {
                    message: error.to_string(),
                })?;
            let existing_skill = existing.map(|skill| ConflictSkillInfo {
                skill_id: skill.id,
                name: skill.name,
                updated_at: skill.audit_fields.updated_at,
                description: skill.description,
            });
            let status = if existing_skill.is_some() {
                CandidateStatus::Conflict
            } else {
                CandidateStatus::Ready
            };
            Ok(ImportCandidate {
                candidate_id,
                name: manifest.name,
                description: manifest.description,
                source_path,
                boundary: boundary.clone(),
                status,
                error_code: None,
                existing_skill,
            })
        }
        Err(error) => Ok(ImportCandidate {
            candidate_id,
            name: String::new(),
            description: String::new(),
            source_path,
            boundary: boundary.clone(),
            status: CandidateStatus::Invalid,
            error_code: Some(manifest_error_code(&error)),
            existing_skill: None,
        }),
    }
}

/// Rejects two valid candidates declaring the same case-insensitive name.
fn reject_duplicate_names(candidates: &[ImportCandidate]) -> Result<(), SkillImportError> {
    let mut by_name: HashMap<String, Vec<&ImportCandidate>> = HashMap::new();
    for candidate in candidates
        .iter()
        .filter(|candidate| candidate.status != CandidateStatus::Invalid)
    {
        by_name
            .entry(candidate.name.to_ascii_lowercase())
            .or_default()
            .push(candidate);
    }
    let duplicates = by_name
        .into_iter()
        .filter(|(_, group)| group.len() > 1)
        .map(|(_, group)| {
            let name = group[0].name.clone();
            let source_paths = group
                .iter()
                .map(|candidate| candidate.source_path.to_string())
                .collect();
            DuplicateSkillName { name, source_paths }
        })
        .collect::<Vec<_>>();
    if duplicates.is_empty() {
        Ok(())
    } else {
        Err(SkillImportError::DuplicateSkillNames { duplicates })
    }
}

/// Reads a manifest with a strict byte cap so oversized manifests are not buffered whole.
///
/// One byte above the limit is enough to prove the manifest exceeds the budget; `parse_manifest`
/// then reports `skill_manifest_too_large`.
fn read_manifest_capped(
    snapshot: &ExtractedTree,
    path: &StrictRelativePath,
    max_bytes: u64,
) -> Result<Vec<u8>, SkillImportError> {
    use std::io::Read;
    let file = fs::File::open(path.to_path(snapshot.root())).map_err(map_io_to_import)?;
    let mut bytes = Vec::with_capacity(max_bytes as usize + 1);
    file.take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(map_io_to_import)?;
    Ok(bytes)
}

/// Copies a raw archive with a streaming size limit before extraction.
fn copy_archive_with_limit(
    source: &Path,
    destination: &Path,
    max_bytes: u64,
) -> Result<(), SkillImportError> {
    let input = fs::File::open(source).map_err(map_io_to_import)?;
    let output = fs::File::create(destination).map_err(map_io_to_import)?;
    let mut counted = CountingWriter {
        inner: output,
        count: 0,
        limit: max_bytes,
    };
    io::copy(&mut &input, &mut counted).map_err(|_| SkillImportError::ArchiveTooLarge)?;
    counted.inner.flush().map_err(map_io_to_import)?;
    Ok(())
}

/// Wraps the destination file to abort once the raw archive size limit is exceeded.
struct CountingWriter<W: Write> {
    inner: W,
    count: u64,
    limit: u64,
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.count + buffer.len() as u64 > self.limit {
            return Err(io::Error::other("archive size limit exceeded"));
        }
        let written = self.inner.write(buffer)?;
        self.count += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Maps one manifest failure onto its stable candidate error code.
fn manifest_error_code(error: &ManifestError) -> String {
    match error {
        ManifestError::YamlInvalid => "yaml_invalid",
        ManifestError::NameMissing => "name_missing",
        ManifestError::NameInvalid => "name_invalid",
        ManifestError::DescriptionMissing => "description_missing",
        ManifestError::DescriptionTooLarge => "description_too_large",
        ManifestError::TooLarge { .. } => "skill_manifest_too_large",
    }
    .to_string()
}

/// Converts a lock poisoning failure into a stable internal error.
fn lock_error<T>(_: std::sync::PoisonError<T>) -> ApplicationError {
    ApplicationError::SkillImport(SkillImportError::Internal {
        message: "import session lock is unavailable".to_string(),
    })
}

/// Converts a session-storage I/O failure into a stable internal error.
fn map_io_error(error: io::Error) -> ApplicationError {
    ApplicationError::SkillImport(SkillImportError::SourceUnavailable {
        message: error.to_string(),
    })
}

/// Converts a session I/O failure into a stable import error.
fn map_io_to_import(error: io::Error) -> SkillImportError {
    SkillImportError::SourceUnavailable {
        message: error.to_string(),
    }
}

/// Converts a duration into the millisecond integer used by the injected clock.
fn duration_millis(duration: Duration) -> i64 {
    duration.as_millis() as i64
}
