use crate::agent_definition::mapper::{map_agent_definition, map_agent_definition_details};
use crate::agent_definition::ports::{AgentDefinitionIdGenerator, AgentDefinitionRepository};
use crate::{ApplicationError, Clock};
use ora_contracts::{
    CreateAgentRequest, CreateAgentResponse, DeleteAgentRequest, DeleteAgentResponse,
    GetAgentRequest, GetAgentResponse, ListAgentsRequest, ListAgentsResponse, UpdateAgentRequest,
    UpdateAgentResponse,
};
use ora_domain::{AgentDefinition, AgentDefinitionId, AuditFields};

/// Handles creation of configurable agent types.
pub struct CreateAgentDefinitionHandler<Repository, IdGenerator, ClockSource> {
    repository: Repository,
    id_generator: IdGenerator,
    clock: ClockSource,
}

impl<Repository, IdGenerator, ClockSource>
    CreateAgentDefinitionHandler<Repository, IdGenerator, ClockSource>
{
    pub fn new(repository: Repository, id_generator: IdGenerator, clock: ClockSource) -> Self {
        Self {
            repository,
            id_generator,
            clock,
        }
    }
}

impl<Repository, IdGenerator, ClockSource>
    CreateAgentDefinitionHandler<Repository, IdGenerator, ClockSource>
where
    Repository: AgentDefinitionRepository,
    IdGenerator: AgentDefinitionIdGenerator,
    ClockSource: Clock,
{
    /// Creates a normalized configurable agent type and returns its public projection.
    pub fn handle(
        &self,
        request: CreateAgentRequest,
    ) -> Result<CreateAgentResponse, ApplicationError> {
        let name = request.name.trim().to_string();
        reject_existing_name(&self.repository, &name)?;

        let now = self.clock.now_timestamp_millis();
        let agent_definition = AgentDefinition::new(
            self.id_generator.generate_agent_definition_id(),
            name,
            request.description,
            request.content.unwrap_or_default(),
            AuditFields::new(now, now, false),
        )
        .map_err(ApplicationError::from_agent_definition_domain_error)?;
        let agent_definition = self
            .repository
            .create_agent_definition(agent_definition)
            .map_err(ApplicationError::from_agent_definition_repository_error)?;

        Ok(CreateAgentResponse {
            agent: map_agent_definition(agent_definition),
        })
    }
}

/// Handles lookup of configurable agent types.
pub struct GetAgentDefinitionHandler<Repository> {
    repository: Repository,
}

impl<Repository> GetAgentDefinitionHandler<Repository> {
    pub fn new(repository: Repository) -> Self {
        Self { repository }
    }
}

impl<Repository> GetAgentDefinitionHandler<Repository>
where
    Repository: AgentDefinitionRepository,
{
    /// Loads one visible configurable agent type or reports not found.
    pub fn handle(&self, request: GetAgentRequest) -> Result<GetAgentResponse, ApplicationError> {
        let agent_id = AgentDefinitionId::new(request.agent_id);
        let agent_definition = self
            .repository
            .find_agent_definition(&agent_id)
            .map_err(ApplicationError::from_agent_definition_repository_error)?
            .ok_or_else(|| ApplicationError::AgentDefinitionNotFound {
                agent_id: agent_id.to_string(),
            })?;

        Ok(GetAgentResponse {
            agent: map_agent_definition_details(agent_definition),
        })
    }
}

/// Handles listing configurable agent types.
pub struct ListAgentDefinitionsHandler<Repository> {
    repository: Repository,
}

impl<Repository> ListAgentDefinitionsHandler<Repository> {
    pub fn new(repository: Repository) -> Self {
        Self { repository }
    }
}

impl<Repository> ListAgentDefinitionsHandler<Repository>
where
    Repository: AgentDefinitionRepository,
{
    /// Lists every visible configurable agent type in deterministic order.
    pub fn handle(
        &self,
        _request: ListAgentsRequest,
    ) -> Result<ListAgentsResponse, ApplicationError> {
        let agents = self
            .repository
            .list_agent_definitions()
            .map_err(ApplicationError::from_agent_definition_repository_error)?;
        Ok(ListAgentsResponse {
            agents: agents.into_iter().map(map_agent_definition).collect(),
        })
    }
}

/// Handles replacement of configurable agent types.
pub struct UpdateAgentDefinitionHandler<Repository, ClockSource> {
    repository: Repository,
    clock: ClockSource,
}

impl<Repository, ClockSource> UpdateAgentDefinitionHandler<Repository, ClockSource> {
    pub fn new(repository: Repository, clock: ClockSource) -> Self {
        Self { repository, clock }
    }
}

impl<Repository, ClockSource> UpdateAgentDefinitionHandler<Repository, ClockSource>
where
    Repository: AgentDefinitionRepository,
    ClockSource: Clock,
{
    /// Replaces editable fields while preserving the agent identifier and creation timestamp.
    pub fn handle(
        &self,
        request: UpdateAgentRequest,
    ) -> Result<UpdateAgentResponse, ApplicationError> {
        let agent_id = AgentDefinitionId::new(request.agent_id);
        let existing = self
            .repository
            .find_agent_definition(&agent_id)
            .map_err(ApplicationError::from_agent_definition_repository_error)?
            .ok_or_else(|| ApplicationError::AgentDefinitionNotFound {
                agent_id: agent_id.to_string(),
            })?;
        let name = request.name.trim().to_string();
        reject_conflicting_name(&self.repository, &name, &existing.id)?;

        let agent_definition = AgentDefinition::new(
            agent_id,
            name,
            request.description,
            request.content.unwrap_or(existing.content),
            AuditFields::new(
                existing.audit_fields.created_at,
                self.clock.now_timestamp_millis(),
                false,
            ),
        )
        .map_err(ApplicationError::from_agent_definition_domain_error)?;
        let agent_definition = self
            .repository
            .update_agent_definition(agent_definition)
            .map_err(ApplicationError::from_agent_definition_repository_error)?;

        Ok(UpdateAgentResponse {
            agent: map_agent_definition(agent_definition),
        })
    }
}

/// Handles soft deletion of configurable agent types.
pub struct DeleteAgentDefinitionHandler<Repository, ClockSource> {
    repository: Repository,
    clock: ClockSource,
}

impl<Repository, ClockSource> DeleteAgentDefinitionHandler<Repository, ClockSource> {
    pub fn new(repository: Repository, clock: ClockSource) -> Self {
        Self { repository, clock }
    }
}

impl<Repository, ClockSource> DeleteAgentDefinitionHandler<Repository, ClockSource>
where
    Repository: AgentDefinitionRepository,
    ClockSource: Clock,
{
    /// Soft-deletes one visible configurable agent type and returns its identifier.
    pub fn handle(
        &self,
        request: DeleteAgentRequest,
    ) -> Result<DeleteAgentResponse, ApplicationError> {
        let agent_id = AgentDefinitionId::new(request.agent_id);
        let deleted = self
            .repository
            .soft_delete_agent_definition(&agent_id, self.clock.now_timestamp_millis())
            .map_err(ApplicationError::from_agent_definition_repository_error)?;
        if !deleted {
            return Err(ApplicationError::AgentDefinitionNotFound {
                agent_id: agent_id.to_string(),
            });
        }

        Ok(DeleteAgentResponse {
            agent_id: agent_id.to_string(),
        })
    }
}
/// Rejects a create whose name collides with any visible agent, case-insensitively.
fn reject_existing_name<Repository: AgentDefinitionRepository>(
    repository: &Repository,
    name: &str,
) -> Result<(), ApplicationError> {
    match repository
        .find_agent_definition_by_name(name)
        .map_err(ApplicationError::from_agent_definition_repository_error)?
    {
        Some(_) => Err(ApplicationError::AgentDefinitionNameConflict {
            name: name.to_string(),
        }),
        None => Ok(()),
    }
}

/// Rejects a rename that would collide with a different visible agent.
fn reject_conflicting_name<Repository: AgentDefinitionRepository>(
    repository: &Repository,
    name: &str,
    own_id: &AgentDefinitionId,
) -> Result<(), ApplicationError> {
    match repository
        .find_agent_definition_by_name(name)
        .map_err(ApplicationError::from_agent_definition_repository_error)?
    {
        Some(other) if &other.id != own_id => Err(ApplicationError::AgentDefinitionNameConflict {
            name: name.to_string(),
        }),
        _ => Ok(()),
    }
}
