//! Agent execution contracts retain author intent independently of installed catalogs.
//!
//! The `agentConfig` wire shape is decoded here as well, so the graph parser only hands each
//! agent node's payload to [`WireAgentConfig::into_model`].

use super::graph::GraphError;
use super::retry::{AgentRetryPolicy, parse_retry_policy};
use ora_domain::PluginId;
use serde::{Deserialize, Deserializer, de::Error};
use std::collections::HashSet;

/// Whether an agent adds a structured variable beside its stable scalar `{node}.output`.
///
/// `Text` and `StructuredTextExposure` remain only to read snapshots written by the previous
/// contract shape; current graphs use `None` for text-only output and `Structured` for both.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentOutputContract {
    /// The node exposes only its stable `{node}.output` variable.
    None,
    /// Legacy spelling for a text-only node; its value is exposed as `{node}.output`.
    Text,
    /// The node exposes raw text as `{node}.output` and a validated object as
    /// `{node}.structured_output`.
    Structured {
        schema: serde_json::Value,
        text_exposure: StructuredTextExposure,
    },
}

/// Legacy structured-output text setting retained only for snapshot decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuredTextExposure {
    /// Only `{node}.structured_output` is written; the raw text is withheld.
    StructuredOnly,
    /// Both `{node}.structured_output` and `{node}.text` are written.
    IncludeFinalText,
}

/// The executable contract of an `agent` node.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentConfig {
    pub executor: AgentExecutor,
    pub role_id: Option<String>,
    pub skills: Vec<AgentSkill>,
    pub mcps: Vec<AgentMcp>,
    pub prompt: String,
    /// When true the node is a persistent interactive session: its first turn pauses at
    /// `Pending` (awaiting input) instead of completing, and the user drives completion.
    pub interactive: bool,
    /// Optional structured parsing performed in addition to persisting the raw output.
    pub output_contract: Option<AgentOutputContract>,
    /// Automatic retry of failed attempts; graphs without `agentConfig.retry` get the default
    /// policy. The Agent runtime ignores it for interactive nodes.
    pub retry: AgentRetryPolicy,
}

/// The agent CLI and model an `agent` node must run with.
///
/// `agent_cli` stays a string here; validating it as an agent identity and checking
/// runtime availability happens in the session driver (phase 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentExecutor {
    pub agent_cli: String,
    pub model_id: String,
}

/// One skill an agent node requires the runtime prompt to invoke when enabled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSkill {
    pub skill_id: String,
    pub enabled: bool,
}

/// One node-local MCP binding; disabled bindings remain portable with the workflow.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMcp {
    pub mcp_id: PluginId,
    pub enabled: bool,
}

/// Rejects ambiguous bindings before a frozen graph can reach the session runtime.
fn deserialize_bindings<'de, D>(deserializer: D) -> Result<Vec<AgentMcp>, D::Error>
where
    D: Deserializer<'de>,
{
    let bindings = Vec::<AgentMcp>::deserialize(deserializer)?;
    let mut ids = HashSet::new();
    for binding in &bindings {
        if !ids.insert(&binding.mcp_id) {
            return Err(D::Error::custom(
                "MCP bindings require unique canonical plugin IDs",
            ));
        }
    }
    Ok(bindings)
}

/// Wire shape of a node's `data.agentConfig`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WireAgentConfig {
    #[serde(default)]
    executor: Option<WireAgentExecutor>,
    #[serde(default)]
    role_id: Option<String>,
    #[serde(default)]
    skills: Vec<WireAgentSkill>,
    #[serde(default, deserialize_with = "deserialize_bindings")]
    mcps: Vec<AgentMcp>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    interactive: Option<bool>,
    output_contract: Option<WireOutputContract>,
    /// Kept as raw JSON so a malformed policy is rejected with a field-level reason instead of
    /// the generic "not valid JSON" a typed serde failure would produce.
    #[serde(default)]
    retry: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireOutputContract {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    text_exposure: Option<String>,
    #[serde(default)]
    schema: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireAgentExecutor {
    #[serde(default)]
    agent_cli: Option<String>,
    #[serde(default)]
    model_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireAgentSkill {
    #[serde(default)]
    skill_id: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
}

impl WireAgentConfig {
    /// Decodes the agent contract of node `node_id`; only an invalid retry policy fails.
    pub(super) fn into_model(self, node_id: &str) -> Result<AgentConfig, GraphError> {
        Ok(AgentConfig {
            executor: AgentExecutor {
                agent_cli: self
                    .executor
                    .as_ref()
                    .and_then(|executor| executor.agent_cli.clone())
                    .unwrap_or_default(),
                model_id: self
                    .executor
                    .as_ref()
                    .and_then(|executor| executor.model_id.clone())
                    .unwrap_or_default(),
            },
            role_id: self.role_id,
            mcps: self.mcps,
            skills: self
                .skills
                .into_iter()
                .map(WireAgentSkill::into_model)
                .collect(),
            prompt: self.prompt.unwrap_or_default(),
            // Missing `interactive` defaults to false so existing graphs stay fully automatic.
            interactive: self.interactive.unwrap_or(false),
            output_contract: self
                .output_contract
                .and_then(WireOutputContract::into_model),
            retry: parse_retry_policy(node_id, self.retry.as_ref())?,
        })
    }
}

impl WireOutputContract {
    /// Maps the wire contract to the domain model; unknown kinds are ignored so future contract
    /// values parse as no contract on older Ora versions.
    fn into_model(self) -> Option<AgentOutputContract> {
        match self.kind.as_deref() {
            Some("none") => Some(AgentOutputContract::None),
            Some("text") => Some(AgentOutputContract::Text),
            Some("structured") => Some(AgentOutputContract::Structured {
                schema: self.schema.unwrap_or_default(),
                // Missing `textExposure` defaults to structured-only so the parsed object is the
                // authoritative variable unless the author opts the raw text back in.
                text_exposure: match self.text_exposure.as_deref() {
                    Some("includeFinalText") => StructuredTextExposure::IncludeFinalText,
                    _ => StructuredTextExposure::StructuredOnly,
                },
            }),
            _ => None,
        }
    }
}

impl WireAgentSkill {
    fn into_model(self) -> AgentSkill {
        AgentSkill {
            skill_id: self.skill_id.unwrap_or_default(),
            // Missing `enabled` defaults to false so skills are never materialized by surprise.
            enabled: self.enabled.unwrap_or(false),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::AgentMcp;
    use crate::WorkflowGraph;
    use ora_domain::PluginId;
    use pretty_assertions::assert_eq;
    use serde_json::{Value, json};

    /// Parses the same wire envelope that publish and import/export persist.
    fn parse_config(config: Value) -> Result<WorkflowGraph, crate::GraphError> {
        WorkflowGraph::parse(&json!({"nodes": [{"id": "agent", "data": {"kind": "agent", "agentConfig": config}}], "edges": []}).to_string())
    }

    /// Both enabled and disabled bindings survive parsing; legacy drafts default to no MCPs.
    #[test]
    fn retains_bindings_and_defaults_legacy_graphs_to_empty() {
        let bindings = json!([{"mcpId": "official/tools", "enabled": true}, {"mcpId": "local/tools", "enabled": false}]);
        let graph = parse_config(json!({"mcps": bindings})).unwrap();
        assert_eq!(
            graph
                .node("agent")
                .unwrap()
                .agent_config
                .as_ref()
                .unwrap()
                .mcps,
            vec![
                AgentMcp {
                    mcp_id: PluginId::parse("official/tools").unwrap(),
                    enabled: true
                },
                AgentMcp {
                    mcp_id: PluginId::parse("local/tools").unwrap(),
                    enabled: false
                },
            ]
        );
        let legacy = parse_config(json!({})).unwrap();
        assert_eq!(
            legacy
                .node("agent")
                .unwrap()
                .agent_config
                .as_ref()
                .unwrap()
                .mcps,
            Vec::<AgentMcp>::new()
        );
    }

    /// Bad author intent is rejected instead of being silently replaced with an empty allowlist.
    #[test]
    fn rejects_ambiguous_or_malformed_bindings() {
        for bindings in [
            json!(null),
            json!({}),
            json!([{"mcpId": " ", "enabled": true}]),
            json!([{"mcpId": "github", "enabled": true}]),
            json!([{"mcpId": "a", "enabled": true}, {"mcpId": "a", "enabled": false}]),
            json!([{"mcpId": 12, "enabled": true}]),
            json!([{"mcpId": "a", "enabled": "true"}]),
            json!([{"mcpId": "a"}]),
        ] {
            assert!(parse_config(json!({"mcps": bindings})).is_err());
        }
    }
}
