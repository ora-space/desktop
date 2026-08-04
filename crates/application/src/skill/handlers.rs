use crate::skill::mapper::map_skill;
use crate::skill::ports::{SkillIdGenerator, SkillPackageStore, SkillRepository};
use crate::{ApplicationError, Clock};
use ora_contracts::{
    CreateSkillRequest, CreateSkillResponse, DeleteSkillRequest, DeleteSkillResponse,
    GetSkillRequest, GetSkillResponse, ListSkillsRequest, ListSkillsResponse, UpdateSkillRequest,
    UpdateSkillResponse,
};
use ora_domain::{AuditFields, Skill, SkillId};

/// Handles creation of a reusable skill definition.
pub struct CreateSkillHandler<Repository, IdGenerator, ClockSource> {
    repository: Repository,
    id_generator: IdGenerator,
    clock: ClockSource,
}

impl<Repository, IdGenerator, ClockSource>
    CreateSkillHandler<Repository, IdGenerator, ClockSource>
{
    pub fn new(repository: Repository, id_generator: IdGenerator, clock: ClockSource) -> Self {
        Self {
            repository,
            id_generator,
            clock,
        }
    }
}

impl<Repository, IdGenerator, ClockSource> CreateSkillHandler<Repository, IdGenerator, ClockSource>
where
    Repository: SkillRepository,
    IdGenerator: SkillIdGenerator,
    ClockSource: Clock,
{
    /// Creates a normalized skill and returns its public projection.
    pub fn handle(
        &self,
        request: CreateSkillRequest,
    ) -> Result<CreateSkillResponse, ApplicationError> {
        let now = self.clock.now_timestamp_millis();
        let skill = Skill::new(
            self.id_generator.generate_skill_id(),
            request.name,
            request.description,
            AuditFields::new(now, now, false),
        )
        .map_err(ApplicationError::from_skill_domain_error)?;
        let skill = self
            .repository
            .create_skill(skill)
            .map_err(ApplicationError::from_skill_repository_error)?;

        Ok(CreateSkillResponse {
            skill: map_skill(skill),
        })
    }
}

/// Handles lookup of one reusable skill definition.
pub struct GetSkillHandler<Repository> {
    repository: Repository,
}

impl<Repository> GetSkillHandler<Repository> {
    pub fn new(repository: Repository) -> Self {
        Self { repository }
    }
}

impl<Repository> GetSkillHandler<Repository>
where
    Repository: SkillRepository,
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

        Ok(GetSkillResponse {
            skill: map_skill(skill),
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

/// Handles replacement of reusable skill definitions.
pub struct UpdateSkillHandler<Repository, ClockSource> {
    repository: Repository,
    clock: ClockSource,
}

impl<Repository, ClockSource> UpdateSkillHandler<Repository, ClockSource> {
    pub fn new(repository: Repository, clock: ClockSource) -> Self {
        Self { repository, clock }
    }
}

impl<Repository, ClockSource> UpdateSkillHandler<Repository, ClockSource>
where
    Repository: SkillRepository,
    ClockSource: Clock,
{
    /// Replaces editable skill fields while preserving its identifier and creation timestamp.
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
        let skill = Skill::new(
            skill_id,
            request.name,
            request.description,
            AuditFields::new(
                existing.audit_fields.created_at,
                self.clock.now_timestamp_millis(),
                false,
            ),
        )
        .map_err(ApplicationError::from_skill_domain_error)?;
        let skill = self
            .repository
            .update_skill(skill)
            .map_err(ApplicationError::from_skill_repository_error)?;

        Ok(UpdateSkillResponse {
            skill: map_skill(skill),
        })
    }
}

/// Handles soft deletion of reusable skill definitions and their imported folders.
pub struct DeleteSkillHandler<Repository, Store, ClockSource> {
    repository: Repository,
    store: Store,
    clock: ClockSource,
}

impl<Repository, Store, ClockSource> DeleteSkillHandler<Repository, Store, ClockSource> {
    pub fn new(repository: Repository, store: Store, clock: ClockSource) -> Self {
        Self {
            repository,
            store,
            clock,
        }
    }
}

impl<Repository, Store, ClockSource> DeleteSkillHandler<Repository, Store, ClockSource>
where
    Repository: SkillRepository,
    Store: SkillPackageStore,
    ClockSource: Clock,
{
    /// Soft-deletes one visible skill, frees its committed folder, and returns its identifier.
    pub fn handle(
        &self,
        request: DeleteSkillRequest,
    ) -> Result<DeleteSkillResponse, ApplicationError> {
        let skill_id = SkillId::new(request.skill_id);
        // Resolve the name before deleting so the committed directory (named after the skill) can
        // be located; a missing visible skill is reported the same way as a soft-delete miss.
        let existing = self
            .repository
            .find_skill(&skill_id)
            .map_err(ApplicationError::from_skill_repository_error)?
            .ok_or_else(|| ApplicationError::SkillNotFound {
                skill_id: skill_id.to_string(),
            })?;
        let deleted = self
            .repository
            .soft_delete_skill(&skill_id, self.clock.now_timestamp_millis())
            .map_err(ApplicationError::from_skill_repository_error)?;
        if !deleted {
            return Err(ApplicationError::SkillNotFound {
                skill_id: skill_id.to_string(),
            });
        }

        // Best-effort folder removal: the catalog row is already the source of truth, and a failure
        // here self-heals on the next startup reconciliation because the row is no longer visible.
        let _ = self.store.remove_committed(&existing.name);

        Ok(DeleteSkillResponse {
            skill_id: skill_id.to_string(),
        })
    }
}
