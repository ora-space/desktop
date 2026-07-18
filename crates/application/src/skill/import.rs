use crate::skill::mapper::map_skill;
use crate::skill::ports::{
    SkillIdGenerator, SkillImportCommitError, SkillImportUnitOfWork, SkillPackageStore,
    SkillPackageStoreError,
};
use crate::skill::validation::{is_safe_skill_name, normalize_relative_path, parse_skill_manifest};
use crate::{ApplicationError, Clock};
use ora_contracts::CreateSkillResponse;
use ora_domain::{AuditFields, Skill, SkillId};
use std::collections::HashSet;
use std::path::PathBuf;

/// Caps the number of files one skill upload may carry to bound staging cost and abuse.
const MAX_SKILL_FILES: usize = 1000;

/// Carries one uploaded file already materialized from transport, ready to stage on disk.
///
/// The web layer owns multipart decoding and hands the application in-memory files so the import
/// use case stays transport-agnostic and unit-testable without touching a real filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadedSkillFile {
    pub relative_path: String,
    pub bytes: Vec<u8>,
}

/// Imports an uploaded skill folder atomically: stage files, parse its manifest, then commit.
///
/// The handler runs two phases. The first stages every file under an `<id>.tmp` directory and
/// resolves the skill from its `SKILL.md`; any failure discards that staging. The second delegates
/// to a unit of work that inserts the row, promotes the directory, and commits together, so the
/// database record and the on-disk folder appear or vanish as one unit.
pub struct ImportSkillHandler<Store, UnitOfWork, IdGenerator, ClockSource> {
    store: Store,
    unit_of_work: UnitOfWork,
    id_generator: IdGenerator,
    clock: ClockSource,
}

impl<Store, UnitOfWork, IdGenerator, ClockSource>
    ImportSkillHandler<Store, UnitOfWork, IdGenerator, ClockSource>
{
    /// Builds the import handler from its filesystem, transaction, identifier, and clock ports.
    pub fn new(
        store: Store,
        unit_of_work: UnitOfWork,
        id_generator: IdGenerator,
        clock: ClockSource,
    ) -> Self {
        Self {
            store,
            unit_of_work,
            id_generator,
            clock,
        }
    }
}

impl<Store, UnitOfWork, IdGenerator, ClockSource>
    ImportSkillHandler<Store, UnitOfWork, IdGenerator, ClockSource>
where
    Store: SkillPackageStore,
    UnitOfWork: SkillImportUnitOfWork,
    IdGenerator: SkillIdGenerator,
    ClockSource: Clock,
{
    /// Validates the upload, stages it, and commits the resolved skill or rolls everything back.
    pub fn handle(
        &self,
        files: Vec<UploadedSkillFile>,
    ) -> Result<CreateSkillResponse, ApplicationError> {
        let staged_paths = validate_upload(&files)?;
        let skill_id = self.id_generator.generate_skill_id();
        self.store
            .create_staging(&skill_id)
            .map_err(storage_error)?;

        // Any preparation failure must discard the staging directory the import just created so a
        // rejected upload never leaves an orphaned `<id>.tmp` behind.
        let skill = match self.stage_and_resolve(&skill_id, &files, &staged_paths) {
            Ok(skill) => skill,
            Err(error) => {
                let _ = self.store.discard_staging(&skill_id);
                return Err(error);
            }
        };

        self.commit(skill_id, skill)
    }

    /// Writes staged files, then resolves the durable skill from its parsed, validated manifest.
    fn stage_and_resolve(
        &self,
        skill_id: &SkillId,
        files: &[UploadedSkillFile],
        staged_paths: &[PathBuf],
    ) -> Result<Skill, ApplicationError> {
        for (file, relative_path) in files.iter().zip(staged_paths) {
            self.store
                .write_file(skill_id, relative_path, &file.bytes)
                .map_err(storage_error)?;
        }

        let manifest_contents = self
            .store
            .read_manifest(skill_id)
            .map_err(storage_error)?
            .ok_or_else(|| ApplicationError::SkillImportInvalid {
                reason: "skill upload is missing a SKILL.md manifest at its root".to_string(),
            })?;
        let manifest = parse_skill_manifest(&manifest_contents)
            .map_err(|reason| ApplicationError::SkillImportInvalid { reason })?;

        if !is_safe_skill_name(&manifest.name) {
            return Err(ApplicationError::SkillImportInvalid {
                reason: format!(
                    "SKILL.md name `{}` is not a valid single-segment directory name",
                    manifest.name
                ),
            });
        }
        if self
            .store
            .committed_exists(&manifest.name)
            .map_err(storage_error)?
        {
            return Err(ApplicationError::SkillFolderConflict {
                name: manifest.name,
            });
        }

        let now = self.clock.now_timestamp_millis();
        Skill::new(
            skill_id.clone(),
            manifest.name,
            manifest.description,
            AuditFields::new(now, now, false),
        )
        .map_err(ApplicationError::from_skill_domain_error)
    }

    /// Runs the insert-then-promote-then-commit unit of work, compensating for each failure point.
    fn commit(
        &self,
        skill_id: SkillId,
        skill: Skill,
    ) -> Result<CreateSkillResponse, ApplicationError> {
        let name = skill.name.clone();
        let promote = || {
            self.store
                .promote(&skill_id, &name)
                .map_err(|SkillPackageStoreError::OperationFailed(message)| message)
        };

        match self.unit_of_work.insert_then(skill.clone(), promote) {
            Ok(()) => Ok(CreateSkillResponse {
                skill: map_skill(skill),
            }),
            Err(SkillImportCommitError::Insert { message }) => {
                let _ = self.store.discard_staging(&skill_id);
                Err(ApplicationError::SkillRepository { message })
            }
            Err(SkillImportCommitError::Promote { message }) => {
                let _ = self.store.discard_staging(&skill_id);
                Err(ApplicationError::SkillPackageStorage { message })
            }
            Err(SkillImportCommitError::CommitAfterPromote { message }) => {
                let _ = self.store.remove_committed(&name);
                Err(ApplicationError::SkillRepository { message })
            }
        }
    }
}

/// Rejects empty, oversized, unsafe, or duplicate-path uploads before any staging directory exists.
fn validate_upload(files: &[UploadedSkillFile]) -> Result<Vec<PathBuf>, ApplicationError> {
    if files.is_empty() {
        return Err(ApplicationError::SkillImportInvalid {
            reason: "skill upload contained no files".to_string(),
        });
    }
    if files.len() > MAX_SKILL_FILES {
        return Err(ApplicationError::SkillImportInvalid {
            reason: format!("skill upload exceeds the {MAX_SKILL_FILES}-file limit"),
        });
    }

    let mut staged_paths = Vec::with_capacity(files.len());
    let mut seen_paths = HashSet::with_capacity(files.len());
    for file in files {
        let relative_path = normalize_relative_path(&file.relative_path).ok_or_else(|| {
            ApplicationError::SkillImportInvalid {
                reason: format!("unsafe upload path `{}`", file.relative_path),
            }
        })?;
        if !seen_paths.insert(relative_path.clone()) {
            return Err(ApplicationError::SkillImportInvalid {
                reason: format!("duplicate upload path `{}`", relative_path.display()),
            });
        }
        staged_paths.push(relative_path);
    }

    Ok(staged_paths)
}

/// Maps skill filesystem failures onto the stable storage application error.
fn storage_error(error: SkillPackageStoreError) -> ApplicationError {
    match error {
        SkillPackageStoreError::OperationFailed(message) => {
            ApplicationError::SkillPackageStorage { message }
        }
    }
}
