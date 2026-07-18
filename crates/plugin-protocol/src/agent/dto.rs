use super::leaf::{
    AgentConfigurationKey, AgentConversationId, AgentCursor, AgentInstallationId, AgentPageLimit,
    AgentPrompt, AgentResourceId, AgentToolCallId, AgentTurnId, ClientRequestId, FiniteJsonNumber,
    HostResolvedAbsolutePath, JsonSafeU64, ProjectHandle, Rfc3339Timestamp, WorktreeHandle,
    deserialize_optional_non_null,
};
use crate::AgentProviderId;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The Host-issued invocation scope, resolved for every request rather than process-global state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
#[ts(export_to = "agent-contract.ts")]
pub enum AgentScope {
    Global {},
    Project {
        project_handle: ProjectHandle,
        working_directory: HostResolvedAbsolutePath,
    },
    Worktree {
        project_handle: ProjectHandle,
        worktree_handle: WorktreeHandle,
        working_directory: HostResolvedAbsolutePath,
    },
}

macro_rules! page_request {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        #[ts(export_to = "agent-contract.ts")]
        pub struct $name {
            pub provider_id: AgentProviderId,
            pub installation_id: AgentInstallationId,
            pub scope: AgentScope,
            #[serde(
                default,
                skip_serializing_if = "Option::is_none",
                deserialize_with = "deserialize_optional_non_null"
            )]
            #[ts(optional)]
            pub cursor: Option<AgentCursor>,
            pub limit: AgentPageLimit,
        }
    };
}

/// Requests discovery of provider installations in a Host-resolved scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "agent-contract.ts")]
pub struct DiscoverInstallationsRequest {
    pub provider_id: AgentProviderId,
    pub scope: AgentScope,
}

/// Requests a redacted configuration summary for one installation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "agent-contract.ts")]
pub struct GetConfigurationSummaryRequest {
    pub provider_id: AgentProviderId,
    pub installation_id: AgentInstallationId,
    pub scope: AgentScope,
}

page_request!(ListSkillsRequest);
page_request!(ListMcpServersRequest);
page_request!(ListConversationsRequest);

/// Starts a conversation and carries the first non-idempotent prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "agent-contract.ts")]
pub struct StartConversationRequest {
    pub provider_id: AgentProviderId,
    pub installation_id: AgentInstallationId,
    pub scope: AgentScope,
    pub client_request_id: ClientRequestId,
    pub prompt: AgentPrompt,
}

/// Sends a non-idempotent prompt to an existing conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "agent-contract.ts")]
pub struct SendMessageRequest {
    pub provider_id: AgentProviderId,
    pub installation_id: AgentInstallationId,
    pub conversation_id: AgentConversationId,
    pub scope: AgentScope,
    pub client_request_id: ClientRequestId,
    pub prompt: AgentPrompt,
}

/// Requests the safety-controlled termination of one active conversation turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "agent-contract.ts")]
pub struct CancelConversationRequest {
    pub provider_id: AgentProviderId,
    pub installation_id: AgentInstallationId,
    pub conversation_id: AgentConversationId,
    pub scope: AgentScope,
}

/// Returns provider installations and non-fatal discovery diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "agent-contract.ts")]
pub struct DiscoverInstallationsResponse {
    pub installations: Vec<AgentInstallation>,
    pub diagnostics: Vec<AgentDiscoveryDiagnostic>,
}

/// A safe display projection of one external Agent installation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "agent-contract.ts")]
pub struct AgentInstallation {
    pub installation_id: AgentInstallationId,
    pub display_name: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_non_null"
    )]
    #[ts(optional)]
    pub version: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_non_null"
    )]
    #[ts(optional)]
    pub location_display: Option<String>,
    pub availability: AgentAvailability,
}

/// Distinguishes an available installation from a display-only unavailable result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
#[ts(export_to = "agent-contract.ts")]
pub enum AgentAvailability {
    Available {},
    Unavailable { reason: String },
}

/// A non-fatal discovery diagnostic with a bounded display message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "agent-contract.ts")]
pub struct AgentDiscoveryDiagnostic {
    pub kind: AgentDiscoveryDiagnosticKind,
    pub message: String,
}

/// The closed v1 set of discovery diagnostic categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent-contract.ts")]
pub enum AgentDiscoveryDiagnosticKind {
    NotFound,
    PermissionDenied,
    ProbeFailed,
}

/// Returns configuration display items without secret values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "agent-contract.ts")]
pub struct GetConfigurationSummaryResponse {
    pub items: Vec<AgentConfigurationItem>,
}

/// One redacted or display-safe configuration item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "agent-contract.ts")]
pub struct AgentConfigurationItem {
    pub key: AgentConfigurationKey,
    pub display_name: String,
    pub source: AgentResourceSource,
    pub value: AgentConfigurationValue,
}

/// The only configuration value shapes allowed to cross the Agent contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
#[ts(export_to = "agent-contract.ts")]
pub enum AgentConfigurationValue {
    Unset {},
    Redacted {},
    Boolean { value: bool },
    Number { value: FiniteJsonNumber },
    String { value: String },
    StringList { value: Vec<String> },
}

/// A bounded page of safe skill summaries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "agent-contract.ts")]
pub struct ListSkillsResponse {
    pub items: Vec<AgentSkillSummary>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_non_null"
    )]
    #[ts(optional)]
    pub next_cursor: Option<AgentCursor>,
}

/// A display-safe skill descriptor without executable content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "agent-contract.ts")]
pub struct AgentSkillSummary {
    pub id: AgentResourceId,
    pub display_name: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_non_null"
    )]
    #[ts(optional)]
    pub description: Option<String>,
    pub source: AgentResourceSource,
}

/// A bounded page of safe MCP server summaries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "agent-contract.ts")]
pub struct ListMcpServersResponse {
    pub items: Vec<AgentMcpServerSummary>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_non_null"
    )]
    #[ts(optional)]
    pub next_cursor: Option<AgentCursor>,
}

/// A display-only MCP server projection that excludes command, environment, and token data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "agent-contract.ts")]
pub struct AgentMcpServerSummary {
    pub id: AgentResourceId,
    pub display_name: String,
    pub transport: AgentMcpTransport,
    pub enabled: bool,
    pub source: AgentResourceSource,
}

/// Identifies where a display resource originated without exposing a path authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
#[ts(export_to = "agent-contract.ts")]
pub enum AgentResourceSource {
    User {},
    Project {},
    Worktree {},
    BuiltIn {},
    Unknown {
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "deserialize_optional_non_null"
        )]
        #[ts(optional)]
        display: Option<String>,
    },
}

/// The safe display categories for MCP transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent-contract.ts")]
pub enum AgentMcpTransport {
    Stdio,
    Http,
    Sse,
    Unknown,
}

/// A bounded page of conversation summaries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "agent-contract.ts")]
pub struct ListConversationsResponse {
    pub items: Vec<AgentConversationSummary>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_non_null"
    )]
    #[ts(optional)]
    pub next_cursor: Option<AgentCursor>,
}

/// A display-safe conversation descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "agent-contract.ts")]
pub struct AgentConversationSummary {
    pub conversation_id: AgentConversationId,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_non_null"
    )]
    #[ts(optional)]
    pub title: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_non_null"
    )]
    #[ts(optional)]
    pub updated_at: Option<Rfc3339Timestamp>,
}

/// The closed stream-event union for conversation methods.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
#[ts(export_to = "agent-contract.ts")]
pub enum AgentEvent {
    ConversationStarted {
        conversation_id: AgentConversationId,
    },
    TextDelta {
        channel: AgentOutputChannel,
        text: String,
    },
    Status {
        phase: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "deserialize_optional_non_null"
        )]
        #[ts(optional)]
        message: Option<String>,
    },
    ToolCall {
        call_id: AgentToolCallId,
        name: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "deserialize_optional_non_null"
        )]
        #[ts(optional)]
        summary: Option<String>,
    },
    ToolResult {
        call_id: AgentToolCallId,
        is_error: bool,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "deserialize_optional_non_null"
        )]
        #[ts(optional)]
        summary: Option<String>,
    },
    Usage {
        usage: AgentUsage,
    },
}

/// Identifies which display channel produced a streamed text delta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent-contract.ts")]
pub enum AgentOutputChannel {
    Assistant,
    Reasoning,
    Tool,
}

/// Optional bounded usage counters; validation requires at least one present field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "agent-contract.ts")]
pub struct AgentUsage {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_non_null"
    )]
    #[ts(optional)]
    pub input_tokens: Option<JsonSafeU64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_non_null"
    )]
    #[ts(optional)]
    pub output_tokens: Option<JsonSafeU64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_non_null"
    )]
    #[ts(optional)]
    pub cost_micros: Option<JsonSafeU64>,
}

/// The terminal result returned by both streaming conversation methods.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "agent-contract.ts")]
pub struct AgentTurnResult {
    pub conversation_id: AgentConversationId,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_non_null"
    )]
    #[ts(optional)]
    pub turn_id: Option<AgentTurnId>,
    pub finish_reason: AgentFinishReason,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_non_null"
    )]
    #[ts(optional)]
    pub usage: Option<AgentUsage>,
}

/// The only successful terminal reasons in Agent Contract v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent-contract.ts")]
pub enum AgentFinishReason {
    Completed,
    Cancelled,
    Limit,
}

/// Proves whether a safety cancellation stopped work or observed it already stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "agent-contract.ts")]
pub struct CancelConversationResponse {
    pub disposition: CancelDisposition,
}

/// The exact success dispositions for the business safety method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent-contract.ts")]
pub enum CancelDisposition {
    Accepted,
    AlreadyStopped,
}

#[cfg(test)]
mod tests {
    use super::{AgentResourceSource, AgentScope, ListSkillsResponse};
    use pretty_assertions::assert_eq;

    /// Rejects explicit null for fields whose contract allows omission only.
    #[test]
    fn rejects_null_optional_fields() {
        assert_eq!(
            serde_json::from_str::<ListSkillsResponse>(r#"{"items":[],"nextCursor":null}"#)
                .is_err(),
            true
        );
        assert_eq!(
            serde_json::from_str::<AgentResourceSource>(r#"{"type":"unknown","display":null}"#)
                .is_err(),
            true
        );
    }

    /// Rejects unknown fields recursively in discriminated object unions.
    #[test]
    fn rejects_unknown_scope_fields() {
        assert_eq!(
            serde_json::from_str::<AgentScope>(r#"{"type":"global","cwd":"D:\\x"}"#).is_err(),
            true
        );
    }
}
