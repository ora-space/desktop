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
    discover_agent,
    DiscoverAgentRequest,
    DiscoverAgentResponse,
    agent_runtime.discover,
    "Discovers what one agent offers in a workspace without creating a session."
);
