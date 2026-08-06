use ora_contracts::{Agent as ContractAgent, AgentDetails};
use ora_domain::AgentDefinition;

/// Projects a domain configurable agent type into its audit-free public contract form.
pub(crate) fn map_agent_definition(agent_definition: AgentDefinition) -> ContractAgent {
    ContractAgent {
        id: agent_definition.id.to_string(),
        name: agent_definition.name,
        description: agent_definition.description,
    }
}

/// Projects one domain agent into the detail form used by the editor.
pub(crate) fn map_agent_definition_details(agent_definition: AgentDefinition) -> AgentDetails {
    AgentDetails {
        id: agent_definition.id.to_string(),
        name: agent_definition.name,
        description: agent_definition.description,
        content: agent_definition.content,
    }
}
