use std::collections::BTreeSet;

use serde::Serialize;

use super::{
    AgentAvailability, AgentBusinessErrorData, AgentConfigurationValue,
    AgentDiscoveryDiagnosticKind, AgentEvent, AgentRequest, AgentResourceSource, AgentResponse,
    AgentTurnResult, AgentUsage, DiscoverInstallationsResponse, GetConfigurationSummaryResponse,
    ListConversationsResponse, ListMcpServersResponse, ListSkillsResponse,
};
use crate::InitializeLimits;

const MAX_DISPLAY_NAME_BYTES: usize = 512;
const MAX_DESCRIPTION_BYTES: usize = 4 * 1024;
const MAX_STATUS_PHASE_BYTES: usize = 512;
const MAX_DISCOVERY_INSTALLATIONS: usize = 128;
const MAX_DISCOVERY_DIAGNOSTICS: usize = 64;
const MAX_CONFIGURATION_ITEMS: usize = 256;
const MAX_STRING_LIST_ITEMS: usize = 128;

/// Stable reasons why a typed Agent value still violates cross-field or container constraints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AgentContractValidationError {
    #[error("Agent value exceeds its negotiated encoded byte limit")]
    EncodedLimit,
    #[error("Agent collection exceeds its item limit")]
    ItemLimit,
    #[error("Agent collection contains a duplicate identity")]
    DuplicateIdentity,
    #[error("Agent display value violates its byte or NUL constraint")]
    InvalidDisplayValue,
    #[error("Agent usage must contain at least one counter")]
    EmptyUsage,
    #[error("empty discovery must include a notFound diagnostic")]
    MissingNotFoundDiagnostic,
    #[error("Agent response does not match the request method")]
    MethodMismatch,
    #[error("Agent request exceeds a negotiated dynamic limit")]
    RequestLimit,
}

impl AgentRequest {
    /// Applies negotiated prompt and page limits that cannot be encoded by fixed leaf newtypes.
    pub fn validate_with_limits(
        &self,
        limits: &InitializeLimits,
    ) -> Result<(), AgentContractValidationError> {
        match self {
            Self::ListSkills(request) => validate_page_request(request.limit.get(), limits),
            Self::ListMcpServers(request) => validate_page_request(request.limit.get(), limits),
            Self::ListConversations(request) => validate_page_request(request.limit.get(), limits),
            Self::StartConversation(request) => {
                validate_prompt(request.prompt.as_str(), limits.max_agent_prompt_bytes)
            }
            Self::SendMessage(request) => {
                validate_prompt(request.prompt.as_str(), limits.max_agent_prompt_bytes)
            }
            Self::DiscoverInstallations(_)
            | Self::GetConfigurationSummary(_)
            | Self::CancelConversation(_) => Ok(()),
        }
    }
}

impl AgentEvent {
    /// Validates event display leaves, usage invariants, and the negotiated encoded byte cap.
    pub fn validate_with_limits(
        &self,
        limits: &InitializeLimits,
    ) -> Result<(), AgentContractValidationError> {
        match self {
            Self::ConversationStarted { .. } => {}
            Self::TextDelta { text, .. } => {
                validate_display(text, limits.max_agent_event_bytes as usize)?;
            }
            Self::Status { phase, message } => {
                validate_display(phase, MAX_STATUS_PHASE_BYTES)?;
                validate_optional_display(message.as_deref(), MAX_DESCRIPTION_BYTES)?;
            }
            Self::ToolCall { name, summary, .. } => {
                validate_display(name, MAX_DISPLAY_NAME_BYTES)?;
                validate_optional_display(summary.as_deref(), MAX_DESCRIPTION_BYTES)?;
            }
            Self::ToolResult { summary, .. } => {
                validate_optional_display(summary.as_deref(), MAX_DESCRIPTION_BYTES)?;
            }
            Self::Usage { usage } => validate_usage(usage)?,
        }
        validate_encoded_limit(self, limits.max_agent_event_bytes)
    }
}

impl AgentTurnResult {
    /// Validates terminal usage and the negotiated result byte cap.
    pub fn validate_with_limits(
        &self,
        limits: &InitializeLimits,
    ) -> Result<(), AgentContractValidationError> {
        if let Some(usage) = &self.usage {
            validate_usage(usage)?;
        }
        validate_encoded_limit(self, limits.max_agent_result_bytes)
    }
}

impl AgentBusinessErrorData {
    /// Bounds the only extension bag by the same negotiated terminal-result budget.
    pub fn validate_with_limits(
        &self,
        limits: &InitializeLimits,
    ) -> Result<(), AgentContractValidationError> {
        validate_encoded_limit(self, limits.max_agent_result_bytes)
    }
}

impl AgentResponse {
    /// Validates the method-specific container, uniqueness, display, and encoded-size contract.
    pub fn validate_for_request(
        &self,
        request: &AgentRequest,
        limits: &InitializeLimits,
    ) -> Result<(), AgentContractValidationError> {
        match (request, self) {
            (AgentRequest::DiscoverInstallations(_), Self::DiscoverInstallations(response)) => {
                validate_discovery(response)?
            }
            (AgentRequest::GetConfigurationSummary(_), Self::GetConfigurationSummary(response)) => {
                validate_configuration(response)?
            }
            (AgentRequest::ListSkills(request), Self::ListSkills(response)) => {
                validate_skills(response, page_cap(request.limit.get(), limits))?;
            }
            (AgentRequest::ListMcpServers(request), Self::ListMcpServers(response)) => {
                validate_mcp_servers(response, page_cap(request.limit.get(), limits))?;
            }
            (AgentRequest::ListConversations(request), Self::ListConversations(response)) => {
                validate_conversations(response, page_cap(request.limit.get(), limits))?;
            }
            (AgentRequest::CancelConversation(_), Self::CancelConversation(_)) => {}
            _ => return Err(AgentContractValidationError::MethodMismatch),
        }
        let value = self
            .to_result_value()
            .map_err(|_| AgentContractValidationError::InvalidDisplayValue)?;
        validate_encoded_limit(&value, limits.max_agent_result_bytes)
    }
}

fn validate_discovery(
    response: &DiscoverInstallationsResponse,
) -> Result<(), AgentContractValidationError> {
    if response.installations.len() > MAX_DISCOVERY_INSTALLATIONS
        || response.diagnostics.len() > MAX_DISCOVERY_DIAGNOSTICS
    {
        return Err(AgentContractValidationError::ItemLimit);
    }
    let mut ids = BTreeSet::new();
    for installation in &response.installations {
        if !ids.insert(&installation.installation_id) {
            return Err(AgentContractValidationError::DuplicateIdentity);
        }
        validate_display(&installation.display_name, MAX_DISPLAY_NAME_BYTES)?;
        validate_optional_display(installation.version.as_deref(), MAX_DISPLAY_NAME_BYTES)?;
        validate_optional_display(
            installation.location_display.as_deref(),
            MAX_DESCRIPTION_BYTES,
        )?;
        if let AgentAvailability::Unavailable { reason } = &installation.availability {
            validate_display(reason, MAX_DESCRIPTION_BYTES)?;
        }
    }
    for diagnostic in &response.diagnostics {
        validate_display(&diagnostic.message, MAX_DESCRIPTION_BYTES)?;
    }
    if response.installations.is_empty()
        && !response
            .diagnostics
            .iter()
            .any(|item| item.kind == AgentDiscoveryDiagnosticKind::NotFound)
    {
        return Err(AgentContractValidationError::MissingNotFoundDiagnostic);
    }
    Ok(())
}

fn validate_configuration(
    response: &GetConfigurationSummaryResponse,
) -> Result<(), AgentContractValidationError> {
    if response.items.len() > MAX_CONFIGURATION_ITEMS {
        return Err(AgentContractValidationError::ItemLimit);
    }
    let mut keys = BTreeSet::new();
    for item in &response.items {
        if !keys.insert(&item.key) {
            return Err(AgentContractValidationError::DuplicateIdentity);
        }
        validate_display(&item.display_name, MAX_DISPLAY_NAME_BYTES)?;
        validate_source(&item.source)?;
        match &item.value {
            AgentConfigurationValue::String { value } => {
                validate_display(value, MAX_DESCRIPTION_BYTES)?;
            }
            AgentConfigurationValue::StringList { value } => {
                if value.len() > MAX_STRING_LIST_ITEMS {
                    return Err(AgentContractValidationError::ItemLimit);
                }
                for element in value {
                    validate_display(element, MAX_DESCRIPTION_BYTES)?;
                }
            }
            AgentConfigurationValue::Unset {}
            | AgentConfigurationValue::Redacted {}
            | AgentConfigurationValue::Boolean { .. }
            | AgentConfigurationValue::Number { .. } => {}
        }
    }
    Ok(())
}

fn validate_skills(
    response: &ListSkillsResponse,
    maximum_items: usize,
) -> Result<(), AgentContractValidationError> {
    if response.items.len() > maximum_items {
        return Err(AgentContractValidationError::ItemLimit);
    }
    let mut ids = BTreeSet::new();
    for item in &response.items {
        if !ids.insert(&item.id) {
            return Err(AgentContractValidationError::DuplicateIdentity);
        }
        validate_display(&item.display_name, MAX_DISPLAY_NAME_BYTES)?;
        validate_optional_display(item.description.as_deref(), MAX_DESCRIPTION_BYTES)?;
        validate_source(&item.source)?;
    }
    Ok(())
}

fn validate_mcp_servers(
    response: &ListMcpServersResponse,
    maximum_items: usize,
) -> Result<(), AgentContractValidationError> {
    if response.items.len() > maximum_items {
        return Err(AgentContractValidationError::ItemLimit);
    }
    let mut ids = BTreeSet::new();
    for item in &response.items {
        if !ids.insert(&item.id) {
            return Err(AgentContractValidationError::DuplicateIdentity);
        }
        validate_display(&item.display_name, MAX_DISPLAY_NAME_BYTES)?;
        validate_source(&item.source)?;
    }
    Ok(())
}

fn validate_conversations(
    response: &ListConversationsResponse,
    maximum_items: usize,
) -> Result<(), AgentContractValidationError> {
    if response.items.len() > maximum_items {
        return Err(AgentContractValidationError::ItemLimit);
    }
    let mut ids = BTreeSet::new();
    for item in &response.items {
        if !ids.insert(&item.conversation_id) {
            return Err(AgentContractValidationError::DuplicateIdentity);
        }
        validate_optional_display(item.title.as_deref(), MAX_DESCRIPTION_BYTES)?;
    }
    Ok(())
}

fn validate_source(source: &AgentResourceSource) -> Result<(), AgentContractValidationError> {
    if let AgentResourceSource::Unknown { display } = source {
        validate_optional_display(display.as_deref(), MAX_DESCRIPTION_BYTES)?;
    }
    Ok(())
}

fn validate_usage(usage: &AgentUsage) -> Result<(), AgentContractValidationError> {
    if usage.input_tokens.is_none() && usage.output_tokens.is_none() && usage.cost_micros.is_none()
    {
        return Err(AgentContractValidationError::EmptyUsage);
    }
    Ok(())
}

fn validate_page_request(
    requested: u8,
    limits: &InitializeLimits,
) -> Result<(), AgentContractValidationError> {
    if u32::from(requested) > limits.max_page_items {
        return Err(AgentContractValidationError::RequestLimit);
    }
    Ok(())
}

fn validate_prompt(prompt: &str, maximum_bytes: u32) -> Result<(), AgentContractValidationError> {
    if prompt.len() > maximum_bytes as usize {
        return Err(AgentContractValidationError::RequestLimit);
    }
    Ok(())
}

fn page_cap(requested: u8, limits: &InitializeLimits) -> usize {
    usize::from(requested).min(limits.max_page_items as usize)
}

fn validate_optional_display(
    value: Option<&str>,
    maximum_bytes: usize,
) -> Result<(), AgentContractValidationError> {
    value.map_or(Ok(()), |value| validate_display(value, maximum_bytes))
}

fn validate_display(value: &str, maximum_bytes: usize) -> Result<(), AgentContractValidationError> {
    if value.len() > maximum_bytes || value.contains('\0') {
        return Err(AgentContractValidationError::InvalidDisplayValue);
    }
    Ok(())
}

fn validate_encoded_limit<T>(
    value: &T,
    maximum_bytes: u32,
) -> Result<(), AgentContractValidationError>
where
    T: Serialize,
{
    let encoded =
        serde_json::to_vec(value).map_err(|_| AgentContractValidationError::InvalidDisplayValue)?;
    if encoded.len() > maximum_bytes as usize {
        return Err(AgentContractValidationError::EncodedLimit);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::AgentContractValidationError;
    use crate::{
        AgentAvailability, AgentInstallation, AgentInstallationId, AgentPageLimit, AgentProviderId,
        AgentRequest, AgentResponse, AgentScope, DiscoverInstallationsRequest,
        DiscoverInstallationsResponse, InitializeLimits, ListSkillsRequest, ListSkillsResponse,
    };
    use pretty_assertions::assert_eq;

    fn provider_id() -> AgentProviderId {
        AgentProviderId::parse("example")
            .unwrap_or_else(|error| panic!("expected provider id: {error}"))
    }

    /// Enforces canonical empty discovery and duplicate installation identity rules.
    #[test]
    fn validates_discovery_container_invariants() {
        let request = AgentRequest::DiscoverInstallations(DiscoverInstallationsRequest {
            provider_id: provider_id(),
            scope: AgentScope::Global {},
        });
        let empty = AgentResponse::DiscoverInstallations(DiscoverInstallationsResponse {
            installations: Vec::new(),
            diagnostics: Vec::new(),
        });
        assert_eq!(
            empty.validate_for_request(&request, &InitializeLimits::v1_defaults()),
            Err(AgentContractValidationError::MissingNotFoundDiagnostic)
        );

        let installation = AgentInstallation {
            installation_id: AgentInstallationId::parse("installation-1")
                .unwrap_or_else(|error| panic!("expected installation id: {error}")),
            display_name: "Agent".to_owned(),
            version: None,
            location_display: None,
            availability: AgentAvailability::Available {},
        };
        let duplicate = AgentResponse::DiscoverInstallations(DiscoverInstallationsResponse {
            installations: vec![installation.clone(), installation],
            diagnostics: Vec::new(),
        });
        assert_eq!(
            duplicate.validate_for_request(&request, &InitializeLimits::v1_defaults()),
            Err(AgentContractValidationError::DuplicateIdentity)
        );
    }

    /// Applies both the caller's page limit and a tightened initialize limit.
    #[test]
    fn validates_dynamic_page_limits() {
        let limits = InitializeLimits::new(8, 4096, 4096, 4096, 4, 1)
            .unwrap_or_else(|error| panic!("expected limits: {error}"));
        let request = AgentRequest::ListSkills(ListSkillsRequest {
            provider_id: provider_id(),
            installation_id: AgentInstallationId::parse("installation-1")
                .unwrap_or_else(|error| panic!("expected installation id: {error}")),
            scope: AgentScope::Global {},
            cursor: None,
            limit: AgentPageLimit::new(2)
                .unwrap_or_else(|error| panic!("expected page limit: {error}")),
        });
        assert_eq!(
            request.validate_with_limits(&limits),
            Err(AgentContractValidationError::RequestLimit)
        );
        assert_eq!(
            AgentResponse::ListSkills(ListSkillsResponse {
                items: Vec::new(),
                next_cursor: None,
            })
            .validate_for_request(&request, &limits),
            Ok(())
        );
    }
}
