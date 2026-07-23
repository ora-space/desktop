use super::dto::{
    CancelConversationRequest, CancelConversationResponse, DiscoverInstallationsRequest,
    DiscoverInstallationsResponse, GetConfigurationSummaryRequest, GetConfigurationSummaryResponse,
    ListConversationsRequest, ListConversationsResponse, ListMcpServersRequest,
    ListMcpServersResponse, ListSkillsRequest, ListSkillsResponse, SendMessageRequest,
    StartConversationRequest,
};
use super::leaf::deserialize_optional_non_null;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ts_rs::TS;

pub const METHOD_DISCOVER_INSTALLATIONS: &str = "agent.discoverInstallations";
pub const METHOD_GET_CONFIGURATION_SUMMARY: &str = "agent.getConfigurationSummary";
pub const METHOD_LIST_SKILLS: &str = "agent.listSkills";
pub const METHOD_LIST_MCP_SERVERS: &str = "agent.listMcpServers";
pub const METHOD_LIST_CONVERSATIONS: &str = "agent.listConversations";
pub const METHOD_START_CONVERSATION: &str = "agent.startConversation";
pub const METHOD_SEND_MESSAGE: &str = "agent.sendMessage";
pub const METHOD_CANCEL_CONVERSATION: &str = "agent.cancelConversation";

/// The exact Agent v1 method registry represented as a closed enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export_to = "agent-contract.ts")]
pub enum AgentMethod {
    #[serde(rename = "agent.discoverInstallations")]
    DiscoverInstallations,
    #[serde(rename = "agent.getConfigurationSummary")]
    GetConfigurationSummary,
    #[serde(rename = "agent.listSkills")]
    ListSkills,
    #[serde(rename = "agent.listMcpServers")]
    ListMcpServers,
    #[serde(rename = "agent.listConversations")]
    ListConversations,
    #[serde(rename = "agent.startConversation")]
    StartConversation,
    #[serde(rename = "agent.sendMessage")]
    SendMessage,
    #[serde(rename = "agent.cancelConversation")]
    CancelConversation,
}

impl AgentMethod {
    /// Returns the exact wire method string owned by the generated registry.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DiscoverInstallations => METHOD_DISCOVER_INSTALLATIONS,
            Self::GetConfigurationSummary => METHOD_GET_CONFIGURATION_SUMMARY,
            Self::ListSkills => METHOD_LIST_SKILLS,
            Self::ListMcpServers => METHOD_LIST_MCP_SERVERS,
            Self::ListConversations => METHOD_LIST_CONVERSATIONS,
            Self::StartConversation => METHOD_START_CONVERSATION,
            Self::SendMessage => METHOD_SEND_MESSAGE,
            Self::CancelConversation => METHOD_CANCEL_CONVERSATION,
        }
    }

    /// Resolves only methods present in Agent Contract v1.
    pub fn from_wire(value: &str) -> Option<Self> {
        match value {
            METHOD_DISCOVER_INSTALLATIONS => Some(Self::DiscoverInstallations),
            METHOD_GET_CONFIGURATION_SUMMARY => Some(Self::GetConfigurationSummary),
            METHOD_LIST_SKILLS => Some(Self::ListSkills),
            METHOD_LIST_MCP_SERVERS => Some(Self::ListMcpServers),
            METHOD_LIST_CONVERSATIONS => Some(Self::ListConversations),
            METHOD_START_CONVERSATION => Some(Self::StartConversation),
            METHOD_SEND_MESSAGE => Some(Self::SendMessage),
            METHOD_CANCEL_CONVERSATION => Some(Self::CancelConversation),
            _ => None,
        }
    }

    /// Returns method-level idempotency, streaming, and safety metadata.
    pub const fn metadata(self) -> AgentMethodMetadata {
        match self {
            Self::DiscoverInstallations
            | Self::GetConfigurationSummary
            | Self::ListSkills
            | Self::ListMcpServers
            | Self::ListConversations => AgentMethodMetadata {
                method: self,
                semantics: InvocationSemantics::Idempotent,
                streaming: false,
                safety_control: false,
            },
            Self::StartConversation | Self::SendMessage => AgentMethodMetadata {
                method: self,
                semantics: InvocationSemantics::NonIdempotent,
                streaming: true,
                safety_control: false,
            },
            Self::CancelConversation => AgentMethodMetadata {
                method: self,
                semantics: InvocationSemantics::Idempotent,
                streaming: false,
                safety_control: true,
            },
        }
    }
}

/// Declares whether a Written request may have an unknowable business outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent-contract.ts")]
pub enum InvocationSemantics {
    Idempotent,
    NonIdempotent,
}

/// Immutable metadata used by both Rust routing and generated SDK drift checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentMethodMetadata {
    pub method: AgentMethod,
    pub semantics: InvocationSemantics,
    pub streaming: bool,
    pub safety_control: bool,
}

/// Every method in deterministic contract order.
pub const ALL_AGENT_METHODS: [AgentMethod; 8] = [
    AgentMethod::DiscoverInstallations,
    AgentMethod::GetConfigurationSummary,
    AgentMethod::ListSkills,
    AgentMethod::ListMcpServers,
    AgentMethod::ListConversations,
    AgentMethod::StartConversation,
    AgentMethod::SendMessage,
    AgentMethod::CancelConversation,
];

/// A typed outbound Agent request; arbitrary method strings cannot be represented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentRequest {
    DiscoverInstallations(DiscoverInstallationsRequest),
    GetConfigurationSummary(GetConfigurationSummaryRequest),
    ListSkills(ListSkillsRequest),
    ListMcpServers(ListMcpServersRequest),
    ListConversations(ListConversationsRequest),
    StartConversation(StartConversationRequest),
    SendMessage(SendMessageRequest),
    CancelConversation(CancelConversationRequest),
}

impl AgentRequest {
    /// Parses params through the exact DTO selected by the closed Agent method registry.
    pub fn from_method_params(
        method: AgentMethod,
        params: Value,
    ) -> Result<Self, serde_json::Error> {
        match method {
            AgentMethod::DiscoverInstallations => {
                serde_json::from_value(params).map(Self::DiscoverInstallations)
            }
            AgentMethod::GetConfigurationSummary => {
                serde_json::from_value(params).map(Self::GetConfigurationSummary)
            }
            AgentMethod::ListSkills => serde_json::from_value(params).map(Self::ListSkills),
            AgentMethod::ListMcpServers => serde_json::from_value(params).map(Self::ListMcpServers),
            AgentMethod::ListConversations => {
                serde_json::from_value(params).map(Self::ListConversations)
            }
            AgentMethod::StartConversation => {
                serde_json::from_value(params).map(Self::StartConversation)
            }
            AgentMethod::SendMessage => serde_json::from_value(params).map(Self::SendMessage),
            AgentMethod::CancelConversation => {
                serde_json::from_value(params).map(Self::CancelConversation)
            }
        }
    }

    /// Returns the fixed registry method corresponding to this typed request.
    pub const fn method(&self) -> AgentMethod {
        match self {
            Self::DiscoverInstallations(_) => AgentMethod::DiscoverInstallations,
            Self::GetConfigurationSummary(_) => AgentMethod::GetConfigurationSummary,
            Self::ListSkills(_) => AgentMethod::ListSkills,
            Self::ListMcpServers(_) => AgentMethod::ListMcpServers,
            Self::ListConversations(_) => AgentMethod::ListConversations,
            Self::StartConversation(_) => AgentMethod::StartConversation,
            Self::SendMessage(_) => AgentMethod::SendMessage,
            Self::CancelConversation(_) => AgentMethod::CancelConversation,
        }
    }

    /// Serializes the exact params object for the JSON-RPC request envelope.
    pub fn to_params_value(&self) -> Result<Value, serde_json::Error> {
        match self {
            Self::DiscoverInstallations(value) => serde_json::to_value(value),
            Self::GetConfigurationSummary(value) => serde_json::to_value(value),
            Self::ListSkills(value) => serde_json::to_value(value),
            Self::ListMcpServers(value) => serde_json::to_value(value),
            Self::ListConversations(value) => serde_json::to_value(value),
            Self::StartConversation(value) => serde_json::to_value(value),
            Self::SendMessage(value) => serde_json::to_value(value),
            Self::CancelConversation(value) => serde_json::to_value(value),
        }
    }
}

/// A typed successful Agent terminal response for non-streaming methods.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentResponse {
    DiscoverInstallations(DiscoverInstallationsResponse),
    GetConfigurationSummary(GetConfigurationSummaryResponse),
    ListSkills(ListSkillsResponse),
    ListMcpServers(ListMcpServersResponse),
    ListConversations(ListConversationsResponse),
    CancelConversation(CancelConversationResponse),
}

impl AgentResponse {
    /// Serializes the typed terminal payload without inventing an application-level union shape.
    pub fn to_result_value(&self) -> Result<Value, serde_json::Error> {
        match self {
            Self::DiscoverInstallations(value) => serde_json::to_value(value),
            Self::GetConfigurationSummary(value) => serde_json::to_value(value),
            Self::ListSkills(value) => serde_json::to_value(value),
            Self::ListMcpServers(value) => serde_json::to_value(value),
            Self::ListConversations(value) => serde_json::to_value(value),
            Self::CancelConversation(value) => serde_json::to_value(value),
        }
    }
}

/// The closed business failure set exposed to plugin authors and Host consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent-contract.ts")]
pub enum AgentBusinessFailureKind {
    AgentUnavailable,
    AuthenticationRequired,
    InvalidAgentConfiguration,
    InstallationNotFound,
    ConversationNotFound,
    UnsupportedAgentCapability,
    InvalidState,
    PermissionDenied,
    CursorExpired,
    AgentProcessFailed,
    ProviderFailure,
}

/// The exact `-32000` error data shape; details are the only bounded extension bag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "agent-contract.ts")]
pub struct AgentBusinessErrorData {
    pub kind: AgentBusinessFailureKind,
    pub retryable: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_non_null"
    )]
    #[ts(optional, type = "Record<string, JsonValue>")]
    pub details: Option<Map<String, Value>>,
}

#[cfg(test)]
mod tests {
    use super::{ALL_AGENT_METHODS, AgentMethod, InvocationSemantics};
    use pretty_assertions::assert_eq;

    /// Freezes method spelling, order, streaming, idempotency, and safety classification together.
    #[test]
    fn exposes_closed_agent_method_registry() {
        let registry = ALL_AGENT_METHODS.map(|method| {
            let metadata = method.metadata();
            (
                method.as_str(),
                metadata.semantics,
                metadata.streaming,
                metadata.safety_control,
            )
        });
        assert_eq!(
            registry,
            [
                (
                    "agent.discoverInstallations",
                    InvocationSemantics::Idempotent,
                    false,
                    false
                ),
                (
                    "agent.getConfigurationSummary",
                    InvocationSemantics::Idempotent,
                    false,
                    false
                ),
                (
                    "agent.listSkills",
                    InvocationSemantics::Idempotent,
                    false,
                    false
                ),
                (
                    "agent.listMcpServers",
                    InvocationSemantics::Idempotent,
                    false,
                    false
                ),
                (
                    "agent.listConversations",
                    InvocationSemantics::Idempotent,
                    false,
                    false
                ),
                (
                    "agent.startConversation",
                    InvocationSemantics::NonIdempotent,
                    true,
                    false
                ),
                (
                    "agent.sendMessage",
                    InvocationSemantics::NonIdempotent,
                    true,
                    false
                ),
                (
                    "agent.cancelConversation",
                    InvocationSemantics::Idempotent,
                    false,
                    true
                ),
            ]
        );
        assert_eq!(AgentMethod::from_wire("agent.unknown"), None);
    }
}
