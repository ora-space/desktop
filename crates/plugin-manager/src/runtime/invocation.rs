use ora_plugin_protocol::{
    AgentBusinessErrorData, AgentEvent, AgentMethod, AgentRequest, AgentResponse, AgentTurnResult,
    CancelConversationResponse, DiscoverInstallationsResponse, ERROR_AGENT_BUSINESS,
    ERROR_REQUEST_CANCELLED, ERROR_SERVER_BUSY, GetConfigurationSummaryResponse, InitializeLimits,
    JsonRpcError, JsonRpcResponse, ListConversationsResponse, ListMcpServersResponse,
    ListSkillsResponse, PluginId,
};
use serde::de::DeserializeOwned;
use tokio::sync::{mpsc, oneshot};

use crate::{AgentContractFailure, PluginError};

/// Typed successful terminal values exposed without leaking transport envelopes.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentInvocationResult {
    Response(AgentResponse),
    Turn(AgentTurnResult),
}

/// Streaming invocation capability with explicit cancellation and one terminal completion.
pub struct AgentInvocationHandle {
    request_id: String,
    events: mpsc::Receiver<AgentEvent>,
    completion: oneshot::Receiver<Result<AgentInvocationResult, PluginError>>,
    cancel: mpsc::Sender<String>,
}

/// Cloneable cancellation-only capability suitable for adapter invocation registries.
#[derive(Clone)]
pub struct AgentInvocationCancellation {
    request_id: String,
    cancel: mpsc::Sender<String>,
}

impl AgentInvocationCancellation {
    /// Requests transport cancellation without exposing the runtime request identifier.
    pub async fn cancel(&self) -> Result<(), PluginError> {
        self.cancel
            .send(self.request_id.clone())
            .await
            .map_err(|_| PluginError::Internal {
                message: "plugin generation is no longer accepting cancellation".to_owned(),
            })
    }
}

impl AgentInvocationHandle {
    pub(crate) fn new(
        request_id: String,
        events: mpsc::Receiver<AgentEvent>,
        completion: oneshot::Receiver<Result<AgentInvocationResult, PluginError>>,
        cancel: mpsc::Sender<String>,
    ) -> Self {
        Self {
            request_id,
            events,
            completion,
            cancel,
        }
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// Derives a cancellation-only capability before the handle is moved into a stream task.
    pub fn cancellation(&self) -> AgentInvocationCancellation {
        AgentInvocationCancellation {
            request_id: self.request_id.clone(),
            cancel: self.cancel.clone(),
        }
    }

    /// Receives one typed stream event; `None` means the stream side is closed.
    pub async fn next_event(&mut self) -> Option<AgentEvent> {
        self.events.recv().await
    }

    /// Requests transport cancellation without claiming it has taken effect remotely.
    pub async fn cancel(&self) -> Result<(), PluginError> {
        self.cancel
            .send(self.request_id.clone())
            .await
            .map_err(|_| PluginError::Internal {
                message: "plugin generation is no longer accepting cancellation".to_owned(),
            })
    }

    /// Waits for the single stable terminal result selected by the runtime actor.
    pub async fn finish(self) -> Result<AgentInvocationResult, PluginError> {
        self.completion.await.unwrap_or_else(|_| {
            Err(PluginError::Internal {
                message: "plugin invocation actor stopped without a terminal result".to_owned(),
            })
        })
    }
}

/// Parses a successful terminal according to the immutable request's closed method registry.
pub(crate) fn parse_agent_success(
    plugin_id: &PluginId,
    request_id: &str,
    request: &AgentRequest,
    value: serde_json::Value,
    limits: &InitializeLimits,
) -> Result<AgentInvocationResult, PluginError> {
    let invalid = || PluginError::AgentContractViolation {
        plugin_id: plugin_id.clone(),
        request_id: request_id.to_owned(),
        reason: AgentContractFailure::InvalidTerminalResult,
    };
    let parsed = match request.method() {
        AgentMethod::DiscoverInstallations => deserialize(value)
            .map(|response: DiscoverInstallationsResponse| {
                AgentInvocationResult::Response(AgentResponse::DiscoverInstallations(response))
            })
            .map_err(|_| invalid()),
        AgentMethod::GetConfigurationSummary => deserialize(value)
            .map(|response: GetConfigurationSummaryResponse| {
                AgentInvocationResult::Response(AgentResponse::GetConfigurationSummary(response))
            })
            .map_err(|_| invalid()),
        AgentMethod::ListSkills => deserialize(value)
            .map(|response: ListSkillsResponse| {
                AgentInvocationResult::Response(AgentResponse::ListSkills(response))
            })
            .map_err(|_| invalid()),
        AgentMethod::ListMcpServers => deserialize(value)
            .map(|response: ListMcpServersResponse| {
                AgentInvocationResult::Response(AgentResponse::ListMcpServers(response))
            })
            .map_err(|_| invalid()),
        AgentMethod::ListConversations => deserialize(value)
            .map(|response: ListConversationsResponse| {
                AgentInvocationResult::Response(AgentResponse::ListConversations(response))
            })
            .map_err(|_| invalid()),
        AgentMethod::StartConversation | AgentMethod::SendMessage => deserialize(value)
            .map(AgentInvocationResult::Turn)
            .map_err(|_| invalid()),
        AgentMethod::CancelConversation => deserialize(value)
            .map(|response: CancelConversationResponse| {
                AgentInvocationResult::Response(AgentResponse::CancelConversation(response))
            })
            .map_err(|_| invalid()),
    }?;
    let validation = match &parsed {
        AgentInvocationResult::Response(response) => response.validate_for_request(request, limits),
        AgentInvocationResult::Turn(result) => result.validate_with_limits(limits),
    };
    validation.map_err(|_| invalid())?;
    Ok(parsed)
}

/// Converts a strict JSON-RPC error into the stable application-facing invocation taxonomy.
pub(crate) fn parse_agent_error(
    plugin_id: &PluginId,
    request_id: &str,
    error: JsonRpcError,
    limits: &InitializeLimits,
) -> PluginError {
    match error.code {
        ERROR_REQUEST_CANCELLED => PluginError::Cancelled {
            plugin_id: plugin_id.clone(),
            request_id: request_id.to_owned(),
        },
        ERROR_SERVER_BUSY => PluginError::PluginBusy {
            plugin_id: plugin_id.clone(),
            request_id: request_id.to_owned(),
        },
        ERROR_AGENT_BUSINESS => {
            let data = error.data.and_then(|data| {
                serde_json::from_value::<AgentBusinessErrorData>(data.into()).ok()
            });
            match data.filter(|data| data.validate_with_limits(limits).is_ok()) {
                Some(data) => PluginError::AgentBusinessFailure {
                    plugin_id: plugin_id.clone(),
                    request_id: request_id.to_owned(),
                    message: error.message,
                    data,
                },
                None => PluginError::AgentContractViolation {
                    plugin_id: plugin_id.clone(),
                    request_id: request_id.to_owned(),
                    reason: AgentContractFailure::InvalidBusinessError,
                },
            }
        }
        _ => PluginError::AgentContractViolation {
            plugin_id: plugin_id.clone(),
            request_id: request_id.to_owned(),
            reason: AgentContractFailure::InvalidTerminalResult,
        },
    }
}

pub(crate) fn parse_agent_terminal(
    plugin_id: &PluginId,
    request_id: &str,
    request: &AgentRequest,
    response: JsonRpcResponse,
    limits: &InitializeLimits,
) -> Result<AgentInvocationResult, PluginError> {
    match response {
        JsonRpcResponse::Success { result, .. } => {
            parse_agent_success(plugin_id, request_id, request, result, limits)
        }
        JsonRpcResponse::Error { error, .. } => {
            Err(parse_agent_error(plugin_id, request_id, error, limits))
        }
    }
}

fn deserialize<T>(value: serde_json::Value) -> Result<T, serde_json::Error>
where
    T: DeserializeOwned,
{
    serde_json::from_value(value)
}
