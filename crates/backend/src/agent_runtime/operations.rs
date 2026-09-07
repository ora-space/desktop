use super::AgentRuntimeManager;
use crate::BackendError;
use ora_contracts::{
    GetAgentRuntimeStatusRequest, GetAgentRuntimeStatusResponse, ListAgentModelsRequest,
    ListAgentModelsResponse,
};
use std::sync::Arc;

/// Reports shared agent readiness and performs on-demand model discovery without exposing actors,
/// supervisor lifecycle control, or session creation to runtime-status consumers.
#[derive(Clone)]
pub struct AgentRuntime {
    manager: Arc<AgentRuntimeManager>,
}

impl AgentRuntime {
    /// Captures the same manager used by sessions, plugins, workflow execution, and Effect.
    pub(crate) fn new(manager: Arc<AgentRuntimeManager>) -> Self {
        Self { manager }
    }

    /// Reports whether each application-scoped CLI runtime is ready, starting, or unavailable.
    pub fn status(
        &self,
        _request: GetAgentRuntimeStatusRequest,
    ) -> Result<GetAgentRuntimeStatusResponse, BackendError> {
        Ok(self.manager.agent_runtime_status())
    }

    /// Discovers one agent's models for a workspace without creating a session.
    pub async fn models(
        &self,
        request: ListAgentModelsRequest,
    ) -> Result<ListAgentModelsResponse, BackendError> {
        self.manager.agent_models(request).await
    }
}
