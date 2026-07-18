use crate::{
    AGENT_CONTRACT_VERSION_V1, AgentContribution, AgentProviderId, ContentOwnerId,
    HostResolvedAbsolutePath, MAX_AGENT_PROMPT_BYTES, MAX_FRAME_BYTES, PluginId, PluginKind,
    PluginVersion,
};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeSet;
use ts_rs::TS;

pub const WIRE_VERSION_V1: u32 = 1;
pub const METHOD_INITIALIZE: &str = "$/initialize";
pub const METHOD_ACTIVATE: &str = "$/activate";
pub const METHOD_DEACTIVATE: &str = "$/deactivate";
pub const METHOD_EXIT: &str = "$/exit";
pub const METHOD_CANCEL_REQUEST: &str = "$/cancelRequest";
pub const METHOD_STREAM: &str = "$/stream";

/// Host-private initialize parameters sent before plugin entry import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "lifecycle.ts")]
pub struct InitializeParams {
    pub wire_version: u32,
    pub host_version: PluginVersion,
    pub runtime_version: PluginVersion,
    pub session_id: String,
    pub plugin: InitializePlugin,
    pub paths: InitializePaths,
    pub declared_agents: Vec<DeclaredAgent>,
    pub limits: InitializeLimits,
}

impl InitializeParams {
    /// Validates cross-field lifecycle invariants before the bootstrap confirms initialization.
    pub fn validate(&self) -> Result<(), LifecycleContractError> {
        if self.wire_version != WIRE_VERSION_V1 {
            return Err(LifecycleContractError::WireVersionMismatch {
                actual: self.wire_version,
            });
        }
        if self.session_id.is_empty() || self.session_id.len() > 128 {
            return Err(LifecycleContractError::InvalidSessionId);
        }
        if self.plugin.kind != PluginKind::Agent
            || self.plugin.plugin_api != crate::PLUGIN_API_VERSION_V1
        {
            return Err(LifecycleContractError::PluginApiMismatch);
        }
        self.limits.validate()?;
        validate_entry_descendant(&self.paths)?;
        validate_declared_agents(&self.declared_agents)
    }
}

/// Immutable identity echoed across the private bootstrap handshake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "lifecycle.ts")]
pub struct InitializePlugin {
    pub id: PluginId,
    pub version: PluginVersion,
    pub kind: PluginKind,
    pub plugin_api: u32,
    pub content_owner: ContentOwnerId,
}

/// Host-derived managed paths that the plugin response cannot replace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "lifecycle.ts")]
pub struct InitializePaths {
    pub extension_path: HostResolvedAbsolutePath,
    pub entry_path: HostResolvedAbsolutePath,
    pub storage_path: HostResolvedAbsolutePath,
}

/// A provider descriptor declared by the immutable manifest.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "lifecycle.ts")]
pub struct DeclaredAgent {
    pub id: AgentProviderId,
    pub contract_version: u32,
}

impl From<&AgentContribution> for DeclaredAgent {
    fn from(contribution: &AgentContribution) -> Self {
        Self {
            id: contribution.id.clone(),
            contract_version: contribution.contract_version,
        }
    }
}

/// The exact seven initialize limits installed by both Host and bootstrap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "lifecycle.ts")]
pub struct InitializeLimits {
    pub max_frame_bytes: u32,
    pub max_pending_requests: u32,
    pub max_agent_event_bytes: u32,
    pub max_agent_result_bytes: u32,
    pub max_agent_prompt_bytes: u32,
    pub max_active_turns: u32,
    pub max_page_items: u32,
}

impl InitializeLimits {
    /// Constructs limits only when every dynamic cap is positive and no larger than wire v1.
    pub fn new(
        max_pending_requests: u32,
        max_agent_event_bytes: u32,
        max_agent_result_bytes: u32,
        max_agent_prompt_bytes: u32,
        max_active_turns: u32,
        max_page_items: u32,
    ) -> Result<Self, LifecycleContractError> {
        let limits = Self {
            max_frame_bytes: MAX_FRAME_BYTES as u32,
            max_pending_requests,
            max_agent_event_bytes,
            max_agent_result_bytes,
            max_agent_prompt_bytes,
            max_active_turns,
            max_page_items,
        };
        limits.validate()?;
        Ok(limits)
    }

    /// Provides the design defaults while preserving the same validation path as configuration.
    pub fn v1_defaults() -> Self {
        Self {
            max_frame_bytes: MAX_FRAME_BYTES as u32,
            max_pending_requests: 128,
            max_agent_event_bytes: 256 * 1024,
            max_agent_result_bytes: 1024 * 1024,
            max_agent_prompt_bytes: MAX_AGENT_PROMPT_BYTES as u32,
            max_active_turns: 64,
            max_page_items: 100,
        }
    }

    /// Applies the exact frame constant and six configurable hard-cap boundaries.
    pub fn validate(&self) -> Result<(), LifecycleContractError> {
        if self.max_frame_bytes != MAX_FRAME_BYTES as u32 {
            return Err(LifecycleContractError::InvalidLimit {
                field: "maxFrameBytes",
                value: self.max_frame_bytes,
                maximum: MAX_FRAME_BYTES as u32,
            });
        }
        validate_limit("maxPendingRequests", self.max_pending_requests, 128)?;
        validate_limit("maxAgentEventBytes", self.max_agent_event_bytes, 256 * 1024)?;
        validate_limit(
            "maxAgentResultBytes",
            self.max_agent_result_bytes,
            1024 * 1024,
        )?;
        validate_limit(
            "maxAgentPromptBytes",
            self.max_agent_prompt_bytes,
            MAX_AGENT_PROMPT_BYTES as u32,
        )?;
        validate_limit("maxActiveTurns", self.max_active_turns, 64)?;
        validate_limit("maxPageItems", self.max_page_items, 100)?;
        Ok(())
    }
}

impl<'de> Deserialize<'de> for InitializeLimits {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct RawInitializeLimits {
            max_frame_bytes: u32,
            max_pending_requests: u32,
            max_agent_event_bytes: u32,
            max_agent_result_bytes: u32,
            max_agent_prompt_bytes: u32,
            max_active_turns: u32,
            max_page_items: u32,
        }

        let raw = RawInitializeLimits::deserialize(deserializer)?;
        let limits = Self {
            max_frame_bytes: raw.max_frame_bytes,
            max_pending_requests: raw.max_pending_requests,
            max_agent_event_bytes: raw.max_agent_event_bytes,
            max_agent_result_bytes: raw.max_agent_result_bytes,
            max_agent_prompt_bytes: raw.max_agent_prompt_bytes,
            max_active_turns: raw.max_active_turns,
            max_page_items: raw.max_page_items,
        };
        limits.validate().map_err(serde::de::Error::custom)?;
        Ok(limits)
    }
}

/// Bootstrap identity echo returned before plugin code can execute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "lifecycle.ts")]
pub struct InitializeResult {
    pub wire_version: u32,
    pub runtime_version: PluginVersion,
    pub session_id: String,
    pub plugin: InitializeResultPlugin,
}

/// The minimal plugin identity echoed by initialize.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "lifecycle.ts")]
pub struct InitializeResultPlugin {
    pub id: PluginId,
    pub version: PluginVersion,
}

/// The only reasons that can trigger the activate lifecycle method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "lifecycle.ts")]
pub enum ActivationReason {
    LazyInvocation,
    ManualStart,
}

/// Exact activate params shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "lifecycle.ts")]
pub struct ActivateParams {
    pub reason: ActivationReason,
}

/// Provider descriptors installed only after successful activation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "lifecycle.ts")]
pub struct ActivateResult {
    pub providers: Vec<DeclaredAgent>,
}

impl ActivateResult {
    /// Requires the activated provider set to equal the manifest set after canonical sorting.
    pub fn validate_declared_providers(
        &self,
        declared: &[DeclaredAgent],
    ) -> Result<(), LifecycleContractError> {
        let mut expected = declared.to_vec();
        expected.sort();
        let mut actual = self.providers.clone();
        actual.sort();
        if actual != expected || actual.windows(2).any(|pair| pair[0].id == pair[1].id) {
            return Err(LifecycleContractError::ProviderMismatch);
        }
        Ok(())
    }
}

/// The reasons for a bounded deactivate request after successful activation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "lifecycle.ts")]
pub enum DeactivationReason {
    ManualStop,
    Disable,
    Uninstall,
    Shutdown,
    GrantChanged,
}

/// Exact deactivate params shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "lifecycle.ts")]
pub struct DeactivateParams {
    pub reason: DeactivationReason,
}

/// Transport cancellation params sent only from Host requester to Plugin responder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "lifecycle.ts")]
pub struct CancelRequestParams {
    pub id: String,
}

/// A typed stream notification tied to one Host request and strict sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "lifecycle.ts")]
pub struct StreamParams {
    pub id: String,
    pub seq: crate::JsonSafeU64,
    pub value: crate::AgentEvent,
}

/// Stable lifecycle DTO validation failures used by handshake diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LifecycleContractError {
    #[error("wireVersion must equal 1, got {actual}")]
    WireVersionMismatch { actual: u32 },
    #[error("sessionId must contain 1..=128 UTF-8 bytes")]
    InvalidSessionId,
    #[error("initialize plugin kind/pluginApi does not identify Agent v1")]
    PluginApiMismatch,
    #[error("declared Agent descriptors are empty, duplicated, or not contractVersion=1")]
    InvalidDeclaredAgents,
    #[error("entryPath must be a normalized strict descendant of extensionPath")]
    InvalidEntryPath,
    #[error("initialize limit {field} must be in 1..={maximum}, got {value}")]
    InvalidLimit {
        field: &'static str,
        value: u32,
        maximum: u32,
    },
    #[error("activate providers do not exactly match manifest declarations")]
    ProviderMismatch,
}

/// Enforces positive, no-greater-than-hard-cap dynamic limits.
fn validate_limit(
    field: &'static str,
    value: u32,
    maximum: u32,
) -> Result<(), LifecycleContractError> {
    if value == 0 || value > maximum {
        return Err(LifecycleContractError::InvalidLimit {
            field,
            value,
            maximum,
        });
    }
    Ok(())
}

/// Rejects duplicate or non-v1 provider descriptors before plugin import.
fn validate_declared_agents(agents: &[DeclaredAgent]) -> Result<(), LifecycleContractError> {
    if agents.is_empty() || agents.len() > 64 {
        return Err(LifecycleContractError::InvalidDeclaredAgents);
    }
    let mut ids = BTreeSet::new();
    if agents.iter().any(|agent| {
        agent.contract_version != AGENT_CONTRACT_VERSION_V1 || !ids.insert(agent.id.clone())
    }) {
        return Err(LifecycleContractError::InvalidDeclaredAgents);
    }
    Ok(())
}

/// Prevents an authored entry path from escaping the Host-resolved extension directory.
fn validate_entry_descendant(paths: &InitializePaths) -> Result<(), LifecycleContractError> {
    let extension = paths
        .extension_path
        .as_str()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase();
    let entry = paths
        .entry_path
        .as_str()
        .replace('/', "\\")
        .to_ascii_lowercase();
    let has_relative_segment = |path: &str| {
        path.split('\\')
            .any(|component| matches!(component, "." | ".."))
    };
    if has_relative_segment(&extension)
        || has_relative_segment(&entry)
        || !entry.starts_with(&format!("{extension}\\"))
    {
        return Err(LifecycleContractError::InvalidEntryPath);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        ActivateResult, DeclaredAgent, InitializeLimits, InitializeParams, InitializePaths,
        InitializePlugin, LifecycleContractError, WIRE_VERSION_V1,
    };
    use crate::{
        AgentProviderId, ContentOwnerId, HostResolvedAbsolutePath, PluginId, PluginKind,
        PluginVersion,
    };
    use pretty_assertions::assert_eq;

    /// Requires the exact seven-field initialize limits and rejects values above hard caps.
    #[test]
    fn validates_initialize_limits() {
        assert_eq!(InitializeLimits::v1_defaults().validate(), Ok(()));
        let invalid = serde_json::from_str::<InitializeLimits>(
            r#"{"maxFrameBytes":8388608,"maxPendingRequests":129,"maxAgentEventBytes":262144,"maxAgentResultBytes":1048576,"maxAgentPromptBytes":1048576,"maxActiveTurns":64,"maxPageItems":100}"#,
        );
        assert!(invalid.is_err());
    }

    /// Compares activated providers as a sorted deep-equal set and still rejects duplicates.
    #[test]
    fn validates_activation_provider_set() {
        let provider = DeclaredAgent {
            id: AgentProviderId::parse("claude-code")
                .unwrap_or_else(|error| panic!("expected provider id: {error}")),
            contract_version: 1,
        };
        assert_eq!(
            ActivateResult {
                providers: vec![provider.clone()],
            }
            .validate_declared_providers(std::slice::from_ref(&provider)),
            Ok(())
        );
        assert_eq!(
            ActivateResult {
                providers: vec![provider.clone(), provider.clone()],
            }
            .validate_declared_providers(&[provider]),
            Err(LifecycleContractError::ProviderMismatch)
        );
    }

    /// Applies nested limit validation and normalized entry containment at the Host boundary.
    #[test]
    fn validates_initialize_cross_field_invariants() {
        let mut params = initialize_params(r"D:\plugins\ora.runtime\dist\index.js");
        assert_eq!(params.validate(), Ok(()));
        params.limits.max_active_turns = 0;
        assert_eq!(
            params.validate(),
            Err(LifecycleContractError::InvalidLimit {
                field: "maxActiveTurns",
                value: 0,
                maximum: 64,
            })
        );
        assert_eq!(
            initialize_params(r"D:\plugins\ora.runtime\..\other\index.js").validate(),
            Err(LifecycleContractError::InvalidEntryPath)
        );
        assert_eq!(
            initialize_params(r"D:\plugins\other\index.js").validate(),
            Err(LifecycleContractError::InvalidEntryPath)
        );
    }

    /// Builds a complete initialize value so validation tests exercise the public entry point.
    fn initialize_params(entry_path: &str) -> InitializeParams {
        InitializeParams {
            wire_version: WIRE_VERSION_V1,
            host_version: PluginVersion::parse("1.0.0")
                .unwrap_or_else(|error| panic!("host version: {error}")),
            runtime_version: PluginVersion::parse("1.0.0")
                .unwrap_or_else(|error| panic!("runtime version: {error}")),
            session_id: "session-1".to_owned(),
            plugin: InitializePlugin {
                id: PluginId::parse("ora.runtime")
                    .unwrap_or_else(|error| panic!("plugin id: {error}")),
                version: PluginVersion::parse("0.1.0")
                    .unwrap_or_else(|error| panic!("plugin version: {error}")),
                kind: PluginKind::Agent,
                plugin_api: crate::PLUGIN_API_VERSION_V1,
                content_owner: ContentOwnerId::parse(format!("sha256-{}", "a".repeat(64)))
                    .unwrap_or_else(|error| panic!("content owner: {error}")),
            },
            paths: InitializePaths {
                extension_path: HostResolvedAbsolutePath::parse(r"D:\plugins\ora.runtime")
                    .unwrap_or_else(|error| panic!("extension path: {error}")),
                entry_path: HostResolvedAbsolutePath::parse(entry_path)
                    .unwrap_or_else(|error| panic!("entry path: {error}")),
                storage_path: HostResolvedAbsolutePath::parse(r"D:\plugin-data\ora.runtime")
                    .unwrap_or_else(|error| panic!("storage path: {error}")),
            },
            declared_agents: vec![DeclaredAgent {
                id: AgentProviderId::parse("example")
                    .unwrap_or_else(|error| panic!("provider id: {error}")),
                contract_version: 1,
            }],
            limits: InitializeLimits::v1_defaults(),
        }
    }
}
