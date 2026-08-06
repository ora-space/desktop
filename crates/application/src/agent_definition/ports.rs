use crate::RepositoryError;
use ora_domain::{AgentDefinition, AgentDefinitionId};

/// Defines persistence operations required by configurable-agent CRUD use cases.
pub trait AgentDefinitionRepository {
    /// Persists a new configurable agent type.
    fn create_agent_definition(
        &self,
        agent_definition: AgentDefinition,
    ) -> Result<AgentDefinition, RepositoryError>;

    /// Loads one visible configurable agent type by identifier.
    fn find_agent_definition(
        &self,
        agent_id: &AgentDefinitionId,
    ) -> Result<Option<AgentDefinition>, RepositoryError>;

    /// Loads the first visible configurable agent whose name matches case-insensitively.
    fn find_agent_definition_by_name(
        &self,
        name: &str,
    ) -> Result<Option<AgentDefinition>, RepositoryError>;

    /// Lists visible configurable agent types in deterministic storage order.
    fn list_agent_definitions(&self) -> Result<Vec<AgentDefinition>, RepositoryError>;

    /// Replaces a visible configurable agent type identified by its stable identifier.
    fn update_agent_definition(
        &self,
        agent_definition: AgentDefinition,
    ) -> Result<AgentDefinition, RepositoryError>;

    /// Marks a visible configurable agent type deleted at the supplied timestamp.
    fn soft_delete_agent_definition(
        &self,
        agent_id: &AgentDefinitionId,
        deleted_at: i64,
    ) -> Result<bool, RepositoryError>;
}

/// Supplies new configurable-agent identifiers for create use cases.
pub trait AgentDefinitionIdGenerator {
    /// Produces the identifier for a newly created configurable agent type.
    fn generate_agent_definition_id(&self) -> AgentDefinitionId;
}
