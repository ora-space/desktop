//! Agent execution contracts retain author intent independently of installed catalogs.
//!
//! The `agentConfig` wire shape is decoded here as well, so the graph parser only hands each
//! agent node's payload to [`WireAgentConfig::into_model`].

use super::graph::GraphError;
use super::retry::{AgentRetryPolicy, parse_retry_policy};
use ora_domain::{PluginId, PromptInactivityPolicy};
use ora_utils::path::RelativeGlob;
use serde::{Deserialize, Deserializer, de::Error};
use std::collections::HashSet;

/// Maximum number of files one node may declare in `agentConfig.completion.files`.
pub const MAX_COMPLETION_FILES: usize = 32;

/// Maximum UTF-8 byte length of one declared completion path or glob.
pub const MAX_COMPLETION_PATH_BYTES: usize = 512;

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

/// Whether an agent node requires an explicit completion signal from the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentCompletionSignal {
    /// The node completes only after the agent calls the host completion tool.
    Agent,
    /// Legacy behavior: a normal `EndTurn` with no blockers is ready.
    #[default]
    EndTurn,
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
    /// Whether the node requires the agent's explicit completion signal.
    pub completion_signal: AgentCompletionSignal,
    /// Whether an accepted signal completes the node without a separate human confirmation.
    pub auto_accept_agent_completion: bool,
    /// Optional structured parsing performed in addition to persisting the raw output.
    pub output_contract: Option<AgentOutputContract>,
    /// Automatic retry of failed attempts; graphs without `agentConfig.retry` get the default
    /// policy. The Agent runtime ignores it for interactive nodes.
    pub retry: AgentRetryPolicy,
    /// Silence handling is frozen with the node so a draft edit cannot interrupt a running turn.
    pub prompt_inactivity: PromptInactivityPolicy,
    /// Optional deliverables checked against the run workspace when the node is completed.
    pub completion: Option<AgentCompletion>,
}

/// One node's declared completion deliverables, frozen with the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCompletion {
    /// Declared file paths or minimal globs, in author order.
    pub files: Vec<CompletionFile>,
}

/// One declared completion file: a workspace-relative glob and whether it must change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionFile {
    /// The validated workspace-relative pattern, preserving the author's declaration.
    pub glob: RelativeGlob,
    /// Whether at least one matching existing file must differ from the node's baseline.
    pub must_change: bool,
}

impl CompletionFile {
    /// Returns the original declaration used in completion blockers and the editor.
    pub fn path(&self) -> &str {
        self.glob.as_str()
    }

    /// Returns whether one workspace-relative path is covered by this declaration.
    pub fn matches(&self, path: &str) -> bool {
        self.glob.matches(path)
    }
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
    /// Kept raw so an invalid version or a version/`completion` mismatch names the node instead of
    /// degrading the whole graph to "not valid JSON".
    #[serde(default)]
    schema_version: Option<serde_json::Value>,
    /// Kept raw so unsupported subfields (`criteria`, `checklist`, typos) are rejected with the
    /// node and field in the reason rather than silently ignored by serde.
    #[serde(default)]
    completion: Option<serde_json::Value>,
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
    #[serde(default)]
    completion_signal: Option<String>,
    #[serde(default)]
    auto_accept_agent_completion: Option<bool>,
    output_contract: Option<WireOutputContract>,
    /// Kept as raw JSON so a malformed policy is rejected with a field-level reason instead of
    /// the generic "not valid JSON" a typed serde failure would produce.
    #[serde(default)]
    retry: Option<serde_json::Value>,
    /// Preserve invalid author input until it can be reported with the owning node and field.
    #[serde(default)]
    prompt_inactivity: Option<serde_json::Value>,
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

/// Wire shape of `agentConfig.completion`; unknown subfields are rejected, never dropped.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireAgentCompletion {
    files: Vec<WireCompletionFile>,
}

/// Wire shape of one `completion.files` entry; `path` stays optional so its own error is readable.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireCompletionFile {
    #[serde(default)]
    path: Option<String>,
    /// The custom deserializer distinguishes an omitted field from an explicit `null`, so a null
    /// declaration is rejected instead of being treated as the default `false`.
    #[serde(default, deserialize_with = "deserialize_optional_bool")]
    must_change: Option<Option<bool>>,
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
    /// Decodes the executable agent contract while rejecting invalid execution policies.
    pub(super) fn into_model(self, node_id: &str) -> Result<AgentConfig, GraphError> {
        let schema_version = parse_schema_version(node_id, self.schema_version.as_ref())?;
        let completion = self
            .completion
            .as_ref()
            .map(|value| parse_completion(node_id, value))
            .transpose()?;
        // The version-4 shape spans completion files, the completion policy and its auto-accept
        // flag; all three follow the same schema boundary as the editor and the TS validator.
        let has_version_4_field = completion.is_some()
            || self.completion_signal.is_some()
            || self.auto_accept_agent_completion.is_some();
        if has_version_4_field && schema_version != Some(4) {
            return Err(GraphError::InvalidNode {
                reason: format!(
                    "node {node_id} declares a version-4 agent completion field, which requires agentConfig.schemaVersion 4 (found {})",
                    schema_version
                        .map_or_else(|| "no version".to_string(), |version| version.to_string())
                ),
            });
        }
        let prompt_inactivity = self
            .prompt_inactivity
            .map(|value| {
                serde_json::from_value::<PromptInactivityPolicy>(value).map_err(|source| {
                    GraphError::InvalidNode {
                        reason: format!("node {node_id} has invalid promptInactivity: {source}"),
                    }
                })
            })
            .transpose()?
            .unwrap_or_default();
        let completion_signal = match self.completion_signal.as_deref() {
            // The graph vocabulary is `end_turn` (see the editor and TS graph types); a missing
            // field is the legacy default, never a new policy.
            None | Some("end_turn") => AgentCompletionSignal::EndTurn,
            Some("agent") => AgentCompletionSignal::Agent,
            Some(other) => {
                return Err(GraphError::InvalidNode {
                    reason: format!("node {node_id} has invalid completionSignal: {other}"),
                });
            }
        };
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
            completion_signal,
            auto_accept_agent_completion: self.auto_accept_agent_completion.unwrap_or(false),
            output_contract: self
                .output_contract
                .and_then(WireOutputContract::into_model),
            retry: parse_retry_policy(node_id, self.retry.as_ref())?,
            prompt_inactivity,
            completion,
        })
    }
}

/// Deserializes a present boolean/null into `Some(value)` while leaving an absent field to
/// `#[serde(default)]`, so the caller can reject an explicit `null` separately from omission.
fn deserialize_optional_bool<'de, D>(deserializer: D) -> Result<Option<Option<bool>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<bool>::deserialize(deserializer).map(Some)
}

/// Validates the optional `agentConfig.schemaVersion` against the versions this Desktop reads.
///
/// A missing version stays the legacy shape. Versions below 3 or above 4 are refused instead of
/// being treated as the nearest known shape: a declaration must never run under semantics it was
/// not written for.
fn parse_schema_version(
    node_id: &str,
    value: Option<&serde_json::Value>,
) -> Result<Option<u32>, GraphError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let version = value.as_u64().ok_or_else(|| GraphError::InvalidNode {
        reason: format!("node {node_id} has a non-integer agentConfig.schemaVersion"),
    })?;
    let version = u32::try_from(version).map_err(|_| GraphError::InvalidNode {
        reason: format!(
            "node {node_id} has unsupported agentConfig.schemaVersion {version}; supported versions are 3 and 4"
        ),
    })?;
    if !(3..=4).contains(&version) {
        return Err(GraphError::InvalidNode {
            reason: format!(
                "node {node_id} has unsupported agentConfig.schemaVersion {version}; supported versions are 3 and 4"
            ),
        });
    }
    Ok(Some(version))
}

/// Decodes and validates `agentConfig.completion`, rejecting unsupported or malformed fields.
fn parse_completion(
    node_id: &str,
    value: &serde_json::Value,
) -> Result<AgentCompletion, GraphError> {
    let wire: WireAgentCompletion =
        serde_json::from_value(value.clone()).map_err(|source| GraphError::InvalidNode {
            reason: format!("node {node_id} has invalid completion: {source}"),
        })?;
    if wire.files.is_empty() || wire.files.len() > MAX_COMPLETION_FILES {
        return Err(GraphError::InvalidNode {
            reason: format!(
                "node {node_id} must declare between 1 and {MAX_COMPLETION_FILES} completion.files entries"
            ),
        });
    }
    let mut files = Vec::with_capacity(wire.files.len());
    for (index, file) in wire.files.into_iter().enumerate() {
        let path = file.path.ok_or_else(|| GraphError::InvalidNode {
            reason: format!("node {node_id} completion.files[{index}].path is required"),
        })?;
        if path.is_empty() {
            return Err(GraphError::InvalidNode {
                reason: format!("node {node_id} completion.files[{index}].path must not be empty"),
            });
        }
        if path.len() > MAX_COMPLETION_PATH_BYTES {
            return Err(GraphError::InvalidNode {
                reason: format!(
                    "node {node_id} completion.files[{index}].path must not exceed {MAX_COMPLETION_PATH_BYTES} UTF-8 bytes"
                ),
            });
        }
        let glob = RelativeGlob::parse(&path).map_err(|source| GraphError::InvalidNode {
            reason: format!(
                "node {node_id} completion.files[{index}].path {path:?} is invalid: {source}"
            ),
        })?;
        let must_change = match file.must_change {
            // Missing `mustChange` defaults to false so a declaration only checks existence.
            None => false,
            Some(Some(value)) => value,
            Some(None) => {
                return Err(GraphError::InvalidNode {
                    reason: format!(
                        "node {node_id} completion.files[{index}].mustChange must be a boolean"
                    ),
                });
            }
        };
        files.push(CompletionFile { glob, must_change });
    }
    Ok(AgentCompletion { files })
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
    use ora_domain::{PluginId, PromptInactivityPolicy};
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

    /// Older snapshots keep timeout semantics while explicit node author intent survives parsing.
    #[test]
    fn prompt_inactivity_defaults_and_explicit_policies_survive_graph_parsing() {
        let legacy = parse_config(json!({})).unwrap();
        let legacy = legacy.node("agent").unwrap().agent_config.as_ref().unwrap();
        assert_eq!(legacy.prompt_inactivity, PromptInactivityPolicy::Timeout);
        for (value, policy) in [
            (json!(null), PromptInactivityPolicy::Timeout),
            (json!("timeout"), PromptInactivityPolicy::Timeout),
            (json!("wait"), PromptInactivityPolicy::Wait),
        ] {
            let graph = parse_config(json!({"promptInactivity": value})).unwrap();
            assert_eq!(
                graph.node("agent").unwrap().agent_config.as_ref().unwrap(),
                &super::AgentConfig {
                    prompt_inactivity: policy,
                    ..legacy.clone()
                },
            );
        }
    }

    /// Missing completion policy keeps legacy end-turn semantics; both explicit values parse.
    #[test]
    fn completion_signal_defaults_to_end_turn_and_parses_both_policies() {
        let legacy = parse_config(json!({})).unwrap();
        let config = legacy.node("agent").unwrap().agent_config.as_ref().unwrap();
        assert_eq!(
            config.completion_signal,
            super::AgentCompletionSignal::EndTurn
        );
        assert!(!config.auto_accept_agent_completion);

        let explicit_end_turn =
            parse_config(json!({"schemaVersion": 4, "completionSignal": "end_turn"})).unwrap();
        let config = explicit_end_turn
            .node("agent")
            .unwrap()
            .agent_config
            .as_ref()
            .unwrap();
        assert_eq!(
            config.completion_signal,
            super::AgentCompletionSignal::EndTurn
        );

        let graph =
            parse_config(json!({"schemaVersion": 4, "completionSignal": "agent", "autoAcceptAgentCompletion": true}))
                .unwrap();
        let config = graph.node("agent").unwrap().agent_config.as_ref().unwrap();
        assert_eq!(
            config.completion_signal,
            super::AgentCompletionSignal::Agent
        );
        assert!(config.auto_accept_agent_completion);
    }

    /// An unknown completion policy is rejected with the node and field context.
    #[test]
    fn rejects_invalid_completion_signal_with_node_context() {
        let error =
            parse_config(json!({"schemaVersion": 4, "completionSignal": "human"})).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("invalid node: node agent has invalid completionSignal: human")
        );
    }

    /// Invalid silence handling cannot be silently replaced by a different execution policy.
    #[test]
    fn rejects_invalid_prompt_inactivity_with_node_and_field_context() {
        for value in [
            json!("disabled"),
            json!(true),
            json!(45),
            json!({}),
            json!([]),
        ] {
            let error = parse_config(json!({"promptInactivity": value})).unwrap_err();
            assert!(
                error
                    .to_string()
                    .starts_with("invalid node: node agent has invalid promptInactivity: ")
            );
        }
    }
    /// A version-4 declaration parses into the execution model without depending on display data.
    #[test]
    fn parses_declared_completion_files_with_their_must_change_flags() {
        let graph = parse_config(json!({
            "schemaVersion": 4,
            "completion": { "files": [
                { "path": "reports/*.md" },
                { "path": "reports/final.md", "mustChange": true }
            ] }
        }))
        .unwrap();
        let completion = graph
            .node("agent")
            .unwrap()
            .agent_config
            .as_ref()
            .unwrap()
            .completion
            .as_ref()
            .unwrap();
        assert_eq!(
            completion.files,
            vec![
                super::CompletionFile {
                    glob: ora_utils::path::RelativeGlob::parse("reports/*.md").unwrap(),
                    must_change: false,
                },
                super::CompletionFile {
                    glob: ora_utils::path::RelativeGlob::parse("reports/final.md").unwrap(),
                    must_change: true,
                },
            ]
        );
        assert!(completion.files[0].matches("reports/plan.md"));
        assert!(!completion.files[0].matches("reports/nested/plan.md"));
    }

    /// The completion policy and its auto-accept flag share the version-4 boundary.
    #[test]
    fn rejects_completion_policy_without_schema_version_4() {
        for config in [
            json!({ "schemaVersion": 3, "completionSignal": "agent" }),
            json!({ "schemaVersion": 3, "autoAcceptAgentCompletion": false }),
        ] {
            let error = parse_config(config).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("requires agentConfig.schemaVersion 4"),
                "{error}"
            );
        }
    }

    /// Declaring `completion` without version 4 is illegal rather than silently downgraded.
    #[test]
    fn rejects_completion_without_schema_version_4() {
        for version in [json!(3), json!(null)] {
            let error = parse_config(json!({
                "schemaVersion": version,
                "completion": { "files": [{ "path": "report.md" }] }
            }))
            .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("requires agentConfig.schemaVersion 4"),
                "{error}"
            );
        }
        // An unsupported version is refused on its own before the combination is considered.
        let error = parse_config(json!({
            "schemaVersion": 2,
            "completion": { "files": [{ "path": "report.md" }] }
        }))
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("unsupported agentConfig.schemaVersion 2"),
            "{error}"
        );
    }

    /// Versions this Desktop does not implement are refused even without a completion declaration.
    #[test]
    fn rejects_unsupported_agent_config_versions() {
        for version in [
            json!(0),
            json!(1),
            json!(2),
            json!(5),
            json!(99),
            json!("3"),
        ] {
            let error = parse_config(json!({ "schemaVersion": version })).unwrap_err();
            assert!(
                error.to_string().contains("agentConfig.schemaVersion"),
                "{version}: {error}"
            );
        }
        // The current version keeps parsing without rewriting legacy graphs.
        assert!(parse_config(json!({ "schemaVersion": 3 })).is_ok());
        assert!(parse_config(json!({ "schemaVersion": 4 })).is_ok());
        assert!(parse_config(json!({})).is_ok());
    }

    /// Unsupported declaration fields fail with the node and field instead of being ignored.
    #[test]
    fn rejects_unsupported_completion_subfields() {
        for extra in [
            json!({ "files": [{ "path": "a.md" }], "criteria": [] }),
            json!({ "files": [{ "path": "a.md" }], "checklist": [] }),
            json!({ "files": [{ "path": "a.md" }], "typo": true }),
            json!({ "files": [{ "path": "a.md", "criteria": [] }] }),
            json!({ "files": [{ "path": "a.md", "must_change": true }] }),
            json!({ "files": [{ "path": "a.md", "mustChange": null }] }),
        ] {
            let error =
                parse_config(json!({ "schemaVersion": 4, "completion": extra })).unwrap_err();
            let text = error.to_string();
            assert!(text.contains("node agent"), "{text}");
            assert!(text.contains("completion"), "{text}");
        }
    }

    /// File counts, path bytes, and path/glob syntax are all bounded at graph parse.
    #[test]
    fn rejects_out_of_bounds_completion_paths_and_file_counts() {
        let cases = [
            json!({ "files": [] }),
            json!({ "files": std::iter::repeat_n(json!({"path":"a.md"}), 33).collect::<Vec<_>>() }),
            json!({ "files": [{ "path": "" }] }),
            json!({ "files": [{ "path": "\u{0}" }] }),
            json!({ "files": [{ "path": "/absolute.md" }] }),
            json!({ "files": [{ "path": "../outside.md" }] }),
            json!({ "files": [{ "path": "a//b.md" }] }),
            json!({ "files": [{ "path": "a\\b.md" }] }),
            json!({ "files": [{ "path": "a/[0-9].md" }] }),
            json!({ "files": [{ "path": "a/{b,c}.md" }] }),
            json!({ "files": [{ "path": "!reports/*.md" }] }),
            json!({ "files": [{ "path": "a/**b/c.md" }] }),
            json!({ "files": [{ "path": "CON" }] }),
            json!({ "files": [{ "path": "a/C:/b.md" }] }),
            json!({ "files": [{ "path": format!("{}/file.md", "a".repeat(256)) }] }),
            json!({ "files": [{ "path": (0..=33).map(|index| format!("d{index}")).collect::<Vec<_>>().join("/") }] }),
            json!({ "files": [{ "path": "x".repeat(super::MAX_COMPLETION_PATH_BYTES + 1) }] }),
        ];
        for completion in cases {
            assert!(
                parse_config(json!({ "schemaVersion": 4, "completion": completion })).is_err(),
                "{completion}"
            );
        }
        // A multibyte path is measured in UTF-8 bytes, not characters.
        let multibyte = "名".repeat(super::MAX_COMPLETION_PATH_BYTES / 3 + 1);
        assert!(
            parse_config(
                json!({ "schemaVersion": 4, "completion": { "files": [{ "path": multibyte }] } })
            )
            .is_err()
        );
    }
}
