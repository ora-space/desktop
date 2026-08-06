use crate::skill::mapper::{map_skill, map_skill_details};
use crate::skill::ports::{SkillIdGenerator, SkillRepository};
use crate::skill::storage::SkillStorage;
use crate::{ApplicationError, Clock};
use gray_matter::{Matter, ParsedEntity, engine::YAML};
use ora_contracts::{
    CreateSkillRequest, CreateSkillResponse, DeleteSkillRequest, DeleteSkillResponse,
    GetSkillRequest, GetSkillResponse, ListSkillsRequest, ListSkillsResponse, UpdateSkillRequest,
    UpdateSkillResponse,
};
use ora_domain::{AuditFields, Skill, SkillId};
use ora_skill_package::manifest::{render_manifest, rewrite_manifest, rewrite_manifest_body};
use serde_json::Value;

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
        reject_existing_name(&self.repository, &name)?;

        let now = self.clock.now_timestamp_millis();
        let skill = Skill::new(
            self.id_generator.generate_skill_id(),
            name,
            request.description,
            AuditFields::new(now, now, false),
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
        let handle = self
            .storage
            .commit_create(&skill.name, &staging)
            .map_err(ApplicationError::from_skill_storage_error)?;
        let created = self.repository.create_skill(skill).map_err(|error| {
            let _ = self.storage.rollback_create(&handle);
            ApplicationError::from_skill_repository_error(error)
        })?;
        self.storage
            .finish_create(&handle)
            .map_err(ApplicationError::from_skill_storage_error)?;

        Ok(CreateSkillResponse {
            skill: map_skill(created),
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
            .map_err(ApplicationError::from_skill_storage_error)?
            .ok_or_else(|| ApplicationError::SkillStorageInconsistent {
                name: skill.name.clone(),
            })?;
        let text = String::from_utf8(manifest).map_err(|_| {
            ApplicationError::from_manifest_error(ora_skill_package::ManifestError::YamlInvalid)
        })?;
        let parsed: ParsedEntity<Value> = Matter::<YAML>::new().parse(&text).map_err(|_| {
            ApplicationError::from_manifest_error(ora_skill_package::ManifestError::YamlInvalid)
        })?;
        Ok(GetSkillResponse {
            skill: map_skill_details(skill, parsed.content),
        })
    }
}

/// Handles listing reusable skill definitions.
pub struct ListSkillsHandler<Repository> {
    repository: Repository,
}

impl<Repository> ListSkillsHandler<Repository> {
    pub fn new(repository: Repository) -> Self {
        Self { repository }
    }
}

impl<Repository> ListSkillsHandler<Repository>
where
    Repository: SkillRepository,
{
    /// Lists every visible skill in the repository's deterministic order.
    pub fn handle(
        &self,
        _request: ListSkillsRequest,
    ) -> Result<ListSkillsResponse, ApplicationError> {
        let skills = self
            .repository
            .list_skills()
            .map_err(ApplicationError::from_skill_repository_error)?;
        Ok(ListSkillsResponse {
            skills: skills.into_iter().map(map_skill).collect(),
        })
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
        reject_conflicting_name(&self.repository, &name, &existing.id)?;

        let skill = Skill::new(
            skill_id,
            name,
            request.description,
            AuditFields::new(
                existing.audit_fields.created_at,
                self.clock.now_timestamp_millis(),
                false,
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
        let handle = self
            .storage
            .commit_swap(&skill.name, &existing.name, &staging)
            .map_err(ApplicationError::from_skill_storage_error)?;
        let updated = self.repository.update_skill(skill).map_err(|error| {
            let _ = self.storage.rollback_swap(&handle);
            ApplicationError::from_skill_repository_error(error)
        })?;
        self.storage
            .finish_swap(&handle)
            .map_err(ApplicationError::from_skill_storage_error)?;

        Ok(UpdateSkillResponse {
            skill: map_skill(updated),
        })
    }
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

        let handle = self
            .storage
            .commit_delete(&existing.name)
            .map_err(ApplicationError::from_skill_storage_error)?;
        let deleted = self
            .repository
            .soft_delete_skill(&skill_id, self.clock.now_timestamp_millis())
            .map_err(|error| {
                let _ = self.storage.rollback_delete(&handle);
                ApplicationError::from_skill_repository_error(error)
            })?;
        if !deleted {
            let _ = self.storage.rollback_delete(&handle);
            return Err(ApplicationError::SkillNotFound {
                skill_id: skill_id.to_string(),
            });
        }
        self.storage
            .finish_delete(&handle)
            .map_err(ApplicationError::from_skill_storage_error)?;

        Ok(DeleteSkillResponse {
            skill_id: skill_id.to_string(),
        })
    }
}

/// Rejects a create whose name collides with any visible skill, case-insensitively.
fn reject_existing_name<Repository: SkillRepository>(
    repository: &Repository,
    name: &str,
) -> Result<(), ApplicationError> {
    match repository
        .find_skill_by_name(name)
        .map_err(ApplicationError::from_skill_repository_error)?
    {
        Some(_) => Err(ApplicationError::SkillNameConflict {
            name: name.to_string(),
        }),
        None => Ok(()),
    }
}

/// Rejects a rename that would collide with a different visible skill.
fn reject_conflicting_name<Repository: SkillRepository>(
    repository: &Repository,
    name: &str,
    own_id: &SkillId,
) -> Result<(), ApplicationError> {
    match repository
        .find_skill_by_name(name)
        .map_err(ApplicationError::from_skill_repository_error)?
    {
        Some(other) if &other.id != own_id => Err(ApplicationError::SkillNameConflict {
            name: name.to_string(),
        }),
        _ => Ok(()),
    }
}
