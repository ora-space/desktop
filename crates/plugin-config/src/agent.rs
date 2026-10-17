//! Static Agent declarations for the platform-owned personal-model access contract.
//!
//! This compiler does not load a user's configuration or authorize a runtime. It is intentionally
//! separate from the installed configuration dispatcher until release and runtime consumers can
//! validate the same declaration identity and negotiate the required model-access contract.

#[cfg(test)]
mod tests;

use crate::declaration::{
    CompileDeclarationError, MAX_DECLARATION_BYTES, SettingDeclaration,
    compile_declaration_from_value, parse_strict_json,
};
use ora_utils::hash::sha256_hex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

const SCHEMA_VERSION: u32 = 2;
const MODEL_ACCESS_CONTRACT_VERSION: u32 = 1;

/// Validated static capabilities and non-secret Setting declarations, never a user's values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledAgentConfiguration {
    pub schema_version: u32,
    pub settings: Vec<SettingDeclaration>,
    pub model_access: AgentModelAccess,
    /// SHA-256 of the exact input bytes, including whitespace; release copies must match the package.
    pub fingerprint: String,
}

/// What a plugin can consume; actual protocol availability and authorization remain host-owned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModelAccess {
    pub contract_version: u32,
    pub protocols: Vec<AgentModelProtocol>,
    pub model_cardinality: AgentModelCardinality,
}

/// Wire protocols, rather than provider names or CLI-specific protocol aliases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentModelProtocol {
    OpenaiCompletions,
    AnthropicMessages,
}

/// A declaration cannot introduce an unapproved second model by adding a role or fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentModelCardinality {
    Single,
}

/// Bounded diagnostics never echo untrusted declaration values into a publisher or host log.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CompileAgentConfigurationError {
    #[error("agent configuration exceeds the {MAX_DECLARATION_BYTES}-byte limit")]
    TooLarge,
    #[error("agent configuration contains a duplicate JSON key")]
    DuplicateKey,
    #[error("agent configuration is not valid JSON")]
    InvalidJson,
    #[error("agent configuration does not match the supported structure")]
    InvalidStructure,
    #[error("unsupported agent configuration schema version {0}")]
    UnsupportedSchemaVersion(u32),
    #[error("unsupported agent model-access contract version {0}")]
    UnsupportedContractVersion(u32),
    #[error("agent configuration requires at least one model protocol")]
    EmptyProtocols,
    #[error("agent configuration declares a duplicate model protocol")]
    DuplicateProtocol,
    #[error("agent configuration declares an unsupported model protocol")]
    UnsupportedProtocol,
    #[error("agent configuration contains invalid Setting declarations")]
    InvalidSettings,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawAgentConfiguration {
    schema_version: u32,
    agent: RawAgentContribution,
    #[serde(default)]
    settings: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawAgentContribution {
    model_access: RawModelAccess,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawModelAccess {
    contract_version: u32,
    protocols: Vec<String>,
    model_cardinality: AgentModelCardinality,
}

/// Compiles a bounded, duplicate-free Agent declaration without I/O or configuration side effects.
///
/// Settings delegate to the existing scalar compiler; its canonical v1 fingerprint is discarded.
/// The Agent declaration's identity covers all of the original bytes, including model capabilities.
pub fn compile_agent_configuration(
    source: &[u8],
) -> Result<CompiledAgentConfiguration, CompileAgentConfigurationError> {
    if source.len() > MAX_DECLARATION_BYTES {
        return Err(CompileAgentConfigurationError::TooLarge);
    }
    let value = parse_strict_json(source).map_err(|error| {
        if error == CompileDeclarationError::DuplicateKey {
            CompileAgentConfigurationError::DuplicateKey
        } else {
            CompileAgentConfigurationError::InvalidJson
        }
    })?;
    // Option<Value> treats explicit null like absence, but a declared Settings member must be valid.
    if value.get("settings").is_some_and(Value::is_null) {
        return Err(CompileAgentConfigurationError::InvalidSettings);
    }
    let raw: RawAgentConfiguration = serde_json::from_value(value)
        .map_err(|_| CompileAgentConfigurationError::InvalidStructure)?;
    if raw.schema_version != SCHEMA_VERSION {
        return Err(CompileAgentConfigurationError::UnsupportedSchemaVersion(
            raw.schema_version,
        ));
    }
    let access = raw.agent.model_access;
    if access.contract_version != MODEL_ACCESS_CONTRACT_VERSION {
        return Err(CompileAgentConfigurationError::UnsupportedContractVersion(
            access.contract_version,
        ));
    }
    if access.protocols.is_empty() {
        return Err(CompileAgentConfigurationError::EmptyProtocols);
    }
    let mut protocols = Vec::with_capacity(access.protocols.len());
    for protocol in access.protocols {
        let protocol = match protocol.as_str() {
            "openai-completions" => AgentModelProtocol::OpenaiCompletions,
            "anthropic-messages" => AgentModelProtocol::AnthropicMessages,
            _ => return Err(CompileAgentConfigurationError::UnsupportedProtocol),
        };
        if protocols.contains(&protocol) {
            return Err(CompileAgentConfigurationError::DuplicateProtocol);
        }
        protocols.push(protocol);
    }
    let settings = match raw.settings {
        Some(settings) => {
            let compiled = compile_declaration_from_value(json!({
                "schemaVersion": 1,
                "settings": settings,
            }))
            .map_err(|_| CompileAgentConfigurationError::InvalidSettings)?;
            compiled.settings
        }
        None => Vec::new(),
    };

    Ok(CompiledAgentConfiguration {
        schema_version: SCHEMA_VERSION,
        settings,
        model_access: AgentModelAccess {
            contract_version: MODEL_ACCESS_CONTRACT_VERSION,
            protocols,
            model_cardinality: access.model_cardinality,
        },
        fingerprint: sha256_hex(source),
    })
}
