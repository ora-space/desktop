//! Desktop agent operations.

use ora_contracts::*;

backend_command!(
    create_agent,
    CreateAgentRequest,
    CreateAgentResponse,
    agents.create,
    "Creates one configurable agent through the shared Backend."
);
backend_command!(
    get_agent,
    GetAgentRequest,
    GetAgentResponse,
    agents.get,
    "Gets one configurable agent through the shared Backend."
);
backend_command!(
    list_agents,
    ListAgentsRequest,
    ListAgentsResponse,
    agents.list,
    "Lists configurable agents through the shared Backend."
);
backend_command!(
    update_agent,
    UpdateAgentRequest,
    UpdateAgentResponse,
    agents.update,
    "Updates one configurable agent through the shared Backend."
);
backend_command!(
    delete_agent,
    DeleteAgentRequest,
    DeleteAgentResponse,
    agents.delete,
    "Deletes one configurable agent through the shared Backend."
);

backend_command!(
    prepare_agent_import,
    PrepareAgentImportRequest,
    PrepareAgentImportResponse,
    agents.prepare_import,
    "Prepares one agent Markdown import source."
);
backend_command!(
    commit_agent_import,
    CommitAgentImportRequest,
    CommitAgentImportResponse,
    agents.commit_import,
    "Commits one prepared agent Markdown import."
);
