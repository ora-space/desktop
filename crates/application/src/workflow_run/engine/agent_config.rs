//! Agent execution contracts retain author intent independently of installed catalogs.

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

/// One skill an agent node declares; only `enabled` skills are materialized at start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSkill {
    pub skill_id: String,
    pub enabled: bool,
}

/// One node-local MCP binding; disabled bindings remain portable with the workflow.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMcp {
    pub mcp_id: String,
    pub enabled: bool,
}

/// Rejects ambiguous bindings before a frozen graph can reach the session runtime.
pub(super) fn deserialize_bindings<'de, D>(deserializer: D) -> Result<Vec<AgentMcp>, D::Error>
where
    D: Deserializer<'de>,
{
    let bindings = Vec::<AgentMcp>::deserialize(deserializer)?;
    let mut ids = HashSet::new();
    for binding in &bindings {
        if binding.mcp_id.trim().is_empty() || !ids.insert(&binding.mcp_id) {
            return Err(D::Error::custom(
                "MCP bindings require nonempty, unique IDs",
            ));
        }
    }
    Ok(bindings)
}

#[cfg(test)]
mod tests {
    use super::AgentMcp;
    use crate::WorkflowGraph;
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
                    mcp_id: "official/tools".into(),
                    enabled: true
                },
                AgentMcp {
                    mcp_id: "local/tools".into(),
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
            json!([{"mcpId": "a", "enabled": true}, {"mcpId": "a", "enabled": false}]),
            json!([{"mcpId": 12, "enabled": true}]),
            json!([{"mcpId": "a", "enabled": "true"}]),
            json!([{"mcpId": "a"}]),
        ] {
            assert!(parse_config(json!({"mcps": bindings})).is_err());
        }
    }
}
