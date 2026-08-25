use crate::skill::mapper::{map_skill, map_skill_details};
use crate::skill::package_health::{
    claim_untracked_name, commit_existing_package, commit_restored_package,
    commit_unclaimed_package, has_usable_package, manifest_is_usable, package_availability,
    persist_promoted_package,
};
use crate::skill::ports::{
    LocalSkillSourceRevision, SkillDeleteOutcome, SkillIdGenerator, SkillRepository,
    SkillSourceInUseError, SkillUpdateOutcome,
};
use crate::skill::storage::{SkillStorage, SkillStorageError};
use crate::{ApplicationError, Clock};
use gray_matter::{Matter, engine::YAML};
use ora_contracts::{
    CreateSkillRequest, CreateSkillResponse, DeleteSkillRequest, DeleteSkillResponse,
    GetSkillRequest, GetSkillResponse, ListSkillsRequest, ListSkillsResponse, SkillAvailability,
    UpdateSkillRequest, UpdateSkillResponse,
};
use ora_domain::{AuditFields, Namespace, Skill, SkillId};
use ora_effect::Digest;
use ora_skill_package::manifest::{render_manifest, rewrite_manifest, rewrite_manifest_body};

/// Handles atomic creation of a reusable skill definition (database plus formal directory).
pub struct CreateSkillHandler<Repository, Storage, IdGenerator, ClockSource> {
    repository: Repository,
    storage: Storage,
    id_generator: IdGenerator,
    clock: ClockSource,
}

impl<Repository, Storage, IdGenerator, ClockSource>
    CreateSkillHandler<Repository, Storage, IdGenerator, ClockSource>
{
    pub fn new(
        repository: Repository,
        storage: Storage,
        id_generator: IdGenerator,
        clock: ClockSource,
    ) -> Self {
        Self {
            repository,
            storage,
            id_generator,
            clock,
        }
    }
}

impl<Repository, Storage, IdGenerator, ClockSource>
    CreateSkillHandler<Repository, Storage, IdGenerator, ClockSource>
where
    Repository: SkillRepository,
    Storage: SkillStorage,
    IdGenerator: SkillIdGenerator,
    ClockSource: Clock,
{
    /// Creates a normalized skill and its minimal manifest atomically.
    pub fn handle(
        &self,
        request: CreateSkillRequest,
    ) -> Result<CreateSkillResponse, ApplicationError> {
        let name = request.name.trim().to_string();
        let namespace = Namespace::local();
        let now = self.clock.now_timestamp_millis();
        if let Some(existing) = self
            .repository
            .find_skill_by_name(&namespace, &name)
            .map_err(ApplicationError::from_skill_repository_error)?
        {
            if has_usable_package(&self.storage, &existing.name)? {
                return Err(ApplicationError::SkillNameConflict {
                    namespace: namespace.to_string(),
                    name,
                });
            }
            let restored = restore_unavailable_skill(
                &self.repository,
                &self.storage,
                existing,
                name,
                request.description,
                request.content.as_deref(),
                now,
            )?;
            return Ok(CreateSkillResponse {
                skill: map_skill(restored, SkillAvailability::Available),
            });
        }

        let skill = Skill::new(
            self.id_generator.generate_skill_id(),
            namespace,
            name,
            request.description,
            AuditFields::new(now, now, /*is_deleted*/ false),
        )
        .map_err(ApplicationError::from_skill_domain_error)?;

        let staging = self
            .storage
            .create_staging()
            .map_err(ApplicationError::from_skill_storage_error)?;
        let manifest = render_manifest(
            &skill.name,
            &skill.description,
            request.content.as_deref().unwrap_or(""),
        );
        self.storage
            .write_manifest(&staging, manifest.as_bytes())
            .map_err(ApplicationError::from_skill_storage_error)?;
        let promoted = commit_unclaimed_package(&self.storage, &skill.id, &skill.name, &staging)?;
        let source_revision = self
            .storage
            .formal_package_path(&skill.name)
            .map(|package_root| LocalSkillSourceRevision {
                skill_md_digest: Digest::sha256(manifest.as_bytes()),
                package_root,
            });
        let created =
            persist_promoted_package(&self.storage, &promoted, || match source_revision {
                Some(source) => self.repository.create_skill_with_source(skill, source),
                None => self.repository.create_skill(skill),
            })
            .map_err(ApplicationError::from_skill_repository_error)?;

        Ok(CreateSkillResponse {
            skill: map_skill(created, SkillAvailability::Available),
        })
    }
}

/// Handles lookup of one reusable skill definition.
pub struct GetSkillHandler<Repository, Storage> {
    repository: Repository,
    storage: Storage,
}

impl<Repository, Storage> GetSkillHandler<Repository, Storage> {
    pub fn new(repository: Repository, storage: Storage) -> Self {
        Self {
            repository,
            storage,
        }
    }
}

impl<Repository, Storage> GetSkillHandler<Repository, Storage>
where
    Repository: SkillRepository,
    Storage: SkillStorage,
{
    /// Loads one visible skill or reports a stable not-found error.
    pub fn handle(&self, request: GetSkillRequest) -> Result<GetSkillResponse, ApplicationError> {
        let skill_id = SkillId::new(request.skill_id);
        let skill = self
            .repository
            .find_skill(&skill_id)
            .map_err(ApplicationError::from_skill_repository_error)?
            .ok_or_else(|| ApplicationError::SkillNotFound {
                skill_id: skill_id.to_string(),
            })?;
        let manifest = self
            .storage
            .read_manifest(&skill.name)
            .map_err(ApplicationError::from_skill_storage_error)?;
        let Some(manifest) = manifest else {
            return Ok(GetSkillResponse {
                skill: map_skill_details(skill, String::new(), SkillAvailability::Unavailable),
            });
        };
        if !manifest_is_usable(&manifest) {
            return Ok(GetSkillResponse {
                skill: map_skill_details(skill, String::new(), SkillAvailability::Unavailable),
            });
        }
        let content = std::str::from_utf8(&manifest)
            .ok()
            .and_then(|text| Matter::<YAML>::new().parse::<serde_json::Value>(text).ok())
            .map(|parsed| parsed.content)
            .unwrap_or_default();
        Ok(GetSkillResponse {
            skill: map_skill_details(skill, content, SkillAvailability::Available),
        })
    }
}

/// Handles listing reusable skill definitions.
pub struct ListSkillsHandler<Repository, Storage> {
    repository: Repository,
    storage: Storage,
}

impl<Repository, Storage> ListSkillsHandler<Repository, Storage> {
    pub fn new(repository: Repository, storage: Storage) -> Self {
        Self {
            repository,
            storage,
        }
    }
}

impl<Repository, Storage> ListSkillsHandler<Repository, Storage>
where
    Repository: SkillRepository,
    Storage: SkillStorage,
{
    /// Lists every visible skill and reports whether its formal package is still loadable.
    pub fn handle(
        &self,
        _request: ListSkillsRequest,
    ) -> Result<ListSkillsResponse, ApplicationError> {
        let skills = self
            .repository
            .list_skills()
            .map_err(ApplicationError::from_skill_repository_error)?;
        let mut mapped = Vec::new();
        for skill in skills {
            let availability = package_availability(&self.storage, &skill.name)?;
            mapped.push(map_skill(skill, availability));
        }
        Ok(ListSkillsResponse { skills: mapped })
    }
}

/// Handles atomic replacement of reusable skill definitions including folder renames.
pub struct UpdateSkillHandler<Repository, Storage, ClockSource> {
    repository: Repository,
    storage: Storage,
    clock: ClockSource,
}

impl<Repository, Storage, ClockSource> UpdateSkillHandler<Repository, Storage, ClockSource> {
    pub fn new(repository: Repository, storage: Storage, clock: ClockSource) -> Self {
        Self {
            repository,
            storage,
            clock,
        }
    }
}

impl<Repository, Storage, ClockSource> UpdateSkillHandler<Repository, Storage, ClockSource>
where
    Repository: SkillRepository,
    Storage: SkillStorage,
    ClockSource: Clock,
{
    /// Replaces editable skill fields while preserving identity, creation time, and package
    /// content that the user did not modify.
    pub fn handle(
        &self,
        request: UpdateSkillRequest,
    ) -> Result<UpdateSkillResponse, ApplicationError> {
        let skill_id = SkillId::new(request.skill_id);
        let existing = self
            .repository
            .find_skill(&skill_id)
            .map_err(ApplicationError::from_skill_repository_error)?
            .ok_or_else(|| ApplicationError::SkillNotFound {
                skill_id: skill_id.to_string(),
            })?;

        let name = request.name.trim().to_string();
        reject_conflicting_name(&self.repository, &existing.namespace, &name, &existing.id)?;
        if !has_usable_package(&self.storage, &existing.name)? {
            let restored = restore_unavailable_skill(
                &self.repository,
                &self.storage,
                existing,
                name,
                request.description,
                request.content.as_deref(),
                self.clock.now_timestamp_millis(),
            )?;
            return Ok(UpdateSkillResponse {
                skill: map_skill(restored, SkillAvailability::Available),
            });
        }

        let previous_updated_at = existing.audit_fields.updated_at;
        let updated_at = next_updated_at(previous_updated_at, self.clock.now_timestamp_millis());
        let skill = Skill::new(
            skill_id,
            existing.namespace,
            name,
            request.description,
            AuditFields::new(
                existing.audit_fields.created_at,
                updated_at,
                /*is_deleted*/ false,
            ),
        )
        .map_err(ApplicationError::from_skill_domain_error)?;

        let staging = self
            .storage
            .create_staging()
            .map_err(ApplicationError::from_skill_storage_error)?;
        // Preserve every package file the user did not modify; only the manifest is rewritten.
        self.storage
            .stage_existing(&existing.name, &staging)
            .map_err(ApplicationError::from_skill_storage_error)?;
        let rewritten = match self
            .storage
            .read_manifest(&existing.name)
            .map_err(ApplicationError::from_skill_storage_error)?
        {
            Some(content) => match request.content.as_deref() {
                Some(body) => {
                    rewrite_manifest_body(&content, &skill.name, &skill.description, body)
                }
                None => rewrite_manifest(&content, &skill.name, &skill.description),
            }
            .map_err(ApplicationError::from_manifest_error)?,
            None => render_manifest(
                &skill.name,
                &skill.description,
                request.content.as_deref().unwrap_or(""),
            ),
        };
        self.storage
            .write_manifest(&staging, rewritten.as_bytes())
            .map_err(ApplicationError::from_skill_storage_error)?;
        let promoted = commit_existing_package(
            &self.storage,
            &skill.id,
            previous_updated_at,
            &skill.name,
            &existing.name,
            &staging,
        )?;
        let source_revision = self
            .storage
            .formal_package_path(&skill.name)
            .map(|package_root| LocalSkillSourceRevision {
                skill_md_digest: Digest::sha256(rewritten.as_bytes()),
                package_root,
            });
        let updated = persist_promoted_package(&self.storage, &promoted, || {
            match source_revision {
                Some(source) => self.repository.update_skill_with_source(skill, source),
                None => self
                    .repository
                    .update_skill(skill)
                    .map(SkillUpdateOutcome::Updated),
            }
            .and_then(|outcome| match outcome {
                SkillUpdateOutcome::Updated(skill) => Ok(skill),
                SkillUpdateOutcome::InUse => {
                    Err(crate::RepositoryError::new(SkillSourceInUseError))
                }
            })
        });
        let updated = match updated {
            Ok(updated) => updated,
            Err(error) if error.is::<SkillSourceInUseError>() => {
                return Err(ApplicationError::SkillInUse);
            }
            Err(error) => return Err(ApplicationError::from_skill_repository_error(error)),
        };

        Ok(UpdateSkillResponse {
            skill: map_skill(updated, SkillAvailability::Available),
        })
    }
}

/// Returns a timestamp that always distinguishes an update from its previous database version.
pub(crate) fn next_updated_at(previous_updated_at: i64, clock_updated_at: i64) -> i64 {
    clock_updated_at.max(previous_updated_at.saturating_add(1))
}

/// Handles atomic soft deletion of reusable skill definitions and their formal directories.
pub struct DeleteSkillHandler<Repository, Storage, ClockSource> {
    repository: Repository,
    storage: Storage,
    clock: ClockSource,
}

impl<Repository, Storage, ClockSource> DeleteSkillHandler<Repository, Storage, ClockSource> {
    pub fn new(repository: Repository, storage: Storage, clock: ClockSource) -> Self {
        Self {
            repository,
            storage,
            clock,
        }
    }
}

impl<Repository, Storage, ClockSource> DeleteSkillHandler<Repository, Storage, ClockSource>
where
    Repository: SkillRepository,
    Storage: SkillStorage,
    ClockSource: Clock,
{
    /// Soft-deletes one visible skill and removes its formal directory atomically.
    pub fn handle(
        &self,
        request: DeleteSkillRequest,
    ) -> Result<DeleteSkillResponse, ApplicationError> {
        let skill_id = SkillId::new(request.skill_id);
        let existing = self
            .repository
            .find_skill(&skill_id)
            .map_err(ApplicationError::from_skill_repository_error)?
            .ok_or_else(|| ApplicationError::SkillNotFound {
                skill_id: skill_id.to_string(),
            })?;

        let handle = match self.storage.commit_delete(&existing.name, &skill_id) {
            Ok(handle) => Some(handle),
            Err(SkillStorageError::FormalDirectoryMissing { .. }) => None,
            Err(error) => return Err(ApplicationError::from_skill_storage_error(error)),
        };
        let deleted = self
            .repository
            .soft_delete_skill_with_source_protection(&skill_id, self.clock.now_timestamp_millis())
            .map_err(|error| {
                if let Some(handle) = &handle {
                    let _ = self.storage.rollback_delete(handle);
                }
                ApplicationError::from_skill_repository_error(error)
            })?;
        if deleted != SkillDeleteOutcome::Deleted {
            if let Some(handle) = &handle {
                let _ = self.storage.rollback_delete(handle);
            }
            if deleted == SkillDeleteOutcome::InUse {
                return Err(ApplicationError::SkillInUse);
            }
            return Err(ApplicationError::SkillNotFound {
                skill_id: skill_id.to_string(),
            });
        }
        if let Some(handle) = handle {
            self.storage
                .finish_delete(&handle)
                .map_err(ApplicationError::from_skill_storage_error)?;
        } else {
            // The catalog row is gone; free the name so a later import is not blocked
            // by a leftover that `exists()` missed when the package was already absent.
            claim_untracked_name(&self.storage, &existing.name)?;
        }

        Ok(DeleteSkillResponse {
            skill_id: skill_id.to_string(),
        })
    }
}

/// Rejects a rename that would collide with a different visible skill.
fn reject_conflicting_name<Repository: SkillRepository>(
    repository: &Repository,
    namespace: &Namespace,
    name: &str,
    own_id: &SkillId,
) -> Result<(), ApplicationError> {
    match repository
        .find_skill_by_name(namespace, name)
        .map_err(ApplicationError::from_skill_repository_error)?
    {
        Some(other) if &other.id != own_id => Err(ApplicationError::SkillNameConflict {
            namespace: namespace.to_string(),
            name: name.to_string(),
        }),
        _ => Ok(()),
    }
}

/// Writes a new formal package onto an unavailable catalog row, preserving its identity.
///
/// Restoring in place copies any leftover package files first so a truncated `SKILL.md`
/// does not destroy sibling scripts; the manifest is then replaced.
fn restore_unavailable_skill<Repository, Storage>(
    repository: &Repository,
    storage: &Storage,
    existing: Skill,
    name: String,
    description: String,
    content: Option<&str>,
    now: i64,
) -> Result<Skill, ApplicationError>
where
    Repository: SkillRepository,
    Storage: SkillStorage,
{
    let previous_updated_at = existing.audit_fields.updated_at;
    let updated_at = next_updated_at(previous_updated_at, now);
    let skill = Skill::new(
        existing.id.clone(),
        existing.namespace.clone(),
        name,
        description,
        AuditFields::new(
            existing.audit_fields.created_at,
            updated_at,
            /*is_deleted*/ false,
        ),
    )
    .map_err(ApplicationError::from_skill_domain_error)?;
    if skill.name != existing.name && storage.formal_exists(&skill.name) {
        return Err(ApplicationError::SkillNameConflict {
            namespace: skill.namespace.to_string(),
            name: skill.name,
        });
    }
    let staging = storage
        .create_staging()
        .map_err(ApplicationError::from_skill_storage_error)?;
    // Preserve the entire residual package for both same-name restore and rename.
    if storage.formal_exists(&existing.name) {
        storage
            .stage_existing(&existing.name, &staging)
            .map_err(ApplicationError::from_skill_storage_error)?;
    }
    let manifest = render_manifest(&skill.name, &skill.description, content.unwrap_or(""));
    storage
        .write_manifest(&staging, manifest.as_bytes())
        .map_err(ApplicationError::from_skill_storage_error)?;
    let promoted = commit_restored_package(
        storage,
        &skill.namespace,
        &skill.id,
        previous_updated_at,
        &skill.name,
        &existing.name,
        &staging,
    )?;
    let source_revision = storage
        .formal_package_path(&skill.name)
        .map(|package_root| LocalSkillSourceRevision {
            skill_md_digest: Digest::sha256(manifest.as_bytes()),
            package_root,
        });
    let updated = persist_promoted_package(storage, &promoted, || {
        match source_revision {
            Some(source) => repository.update_skill_with_source(skill, source),
            None => repository
                .update_skill(skill)
                .map(SkillUpdateOutcome::Updated),
        }
        .and_then(|outcome| match outcome {
            SkillUpdateOutcome::Updated(skill) => Ok(skill),
            SkillUpdateOutcome::InUse => Err(crate::RepositoryError::new(SkillSourceInUseError)),
        })
    });
    match updated {
        Ok(skill) => Ok(skill),
        Err(error) if error.is::<SkillSourceInUseError>() => Err(ApplicationError::SkillInUse),
        Err(error) => Err(ApplicationError::from_skill_repository_error(error)),
    }
}
