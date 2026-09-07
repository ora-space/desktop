//! Desktop agent runtime operations.

use ora_contracts::*;

backend_command!(
    get_agent_runtime_status,
    GetAgentRuntimeStatusRequest,
    GetAgentRuntimeStatusResponse,
    agent_runtime.status,
    "Reports readiness of the shared agent runtimes."
);
async_backend_command!(
    list_agent_models,
    ListAgentModelsRequest,
    ListAgentModelsResponse,
    agent_runtime.models,
    "Discovers one agent's models for a workspace without creating a session."
);
