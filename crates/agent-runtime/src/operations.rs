use super::{AgentRuntimeHost, AgentRuntimeManager};
use crate::RuntimeError;
use ora_contracts::{
    DiscoverAgentRequest, DiscoverAgentResponse, GetAgentRuntimeStatusRequest,
    GetAgentRuntimeStatusResponse,
};
use std::sync::Arc;

/// Reports shared agent readiness and performs on-demand model discovery without exposing actors,
/// supervisor lifecycle control, or session creation to runtime-status consumers.
pub struct AgentRuntime<H: AgentRuntimeHost> {
    manager: Arc<AgentRuntimeManager<H>>,
}

impl<H: AgentRuntimeHost> Clone for AgentRuntime<H> {
    fn clone(&self) -> Self {
        Self {
            manager: Arc::clone(&self.manager),
        }
    }
}

impl<H: AgentRuntimeHost> AgentRuntime<H> {
    /// Captures the same manager used by sessions, plugins, workflow execution, and Effect.
    pub fn new(manager: Arc<AgentRuntimeManager<H>>) -> Self {
        Self { manager }
    }

    /// Reports whether each application-scoped CLI runtime is ready, starting, or unavailable.
    pub fn status(
        &self,
        _request: GetAgentRuntimeStatusRequest,
    ) -> Result<GetAgentRuntimeStatusResponse, RuntimeError> {
        Ok(self.manager.agent_runtime_status())
    }

    /// Discovers what one agent offers in a workspace without creating a session.
    pub async fn discover(
        &self,
        request: DiscoverAgentRequest,
    ) -> Result<DiscoverAgentResponse, RuntimeError> {
        self.manager.discover_agent(request).await
    }
}
