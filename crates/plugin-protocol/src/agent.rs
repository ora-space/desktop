use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use ts_rs::TS;

/// Method that starts the agent process owned by a plugin.
pub const AGENT_START_METHOD: &str = "agent/start";
/// Method that stops the agent while leaving its plugin process alive.
pub const AGENT_STOP_METHOD: &str = "agent/stop";
/// Method that reports an agent's models and commands before an ACP session exists.
pub const AGENT_DISCOVER_METHOD: &str = "agent/discover";
/// Method plugins built before [`AGENT_DISCOVER_METHOD`] answer with models alone.
pub const AGENT_LIST_MODELS_METHOD: &str = "agent/list_models";
/// Bidirectional notification that carries one opaque ACP frame.
pub const AGENT_ACP_METHOD: &str = "agent/acp";

/// Error returned when the agent executable is not installed on the machine.
pub const AGENT_NOT_INSTALLED_CODE: i64 = -32001;
/// Error returned when the executable bundled by an agent package cannot run.
pub const AGENT_UNUSABLE_CODE: i64 = -32002;
/// How long the host gives one discovery.
///
/// The host states when that ends in [`AgentDiscoverParams::deadline`], which is what a plugin goes
/// by. It is published only for [`AGENT_LIST_MODELS_METHOD`], whose params carry no deadline.
pub const AGENT_DISCOVERY_BUDGET_MS: u64 = 60_000;
/// Longest a plugin may wait for an agent's first command report, from its `session/new` response.
///
/// Every plugin may lower it and none may raise it, so one slow agent cannot hold up the pickers.
pub const AGENT_COMMANDS_WAIT_CAP_MS: u64 = 2_000;
/// Key of the ACP session setup `_meta` that lists the directories of the Skills Ora makes available.
pub const SKILL_DIRECTORIES_META_KEY: &str = "ora/skillDirectories";
/// ACP major version carried over the agent plugin channel.
pub const SUPPORTED_ACP_VERSION: u32 = 1;
/// OpenCode shared project configuration materialization format.
pub const OPENCODE_MCP_CONFIG_V1: &str = "ora/opencode-mcp-config.v1";
/// Claude shared project configuration materialization format.
pub const CLAUDE_MCP_CONFIG_V1: &str = "ora/claude-mcp-config.v1";

/// Host context handed to an agent when its underlying process starts.
#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent.ts")]
pub struct AgentStartContext {
    #[ts(type = "string")]
    pub cwd: PathBuf,
    pub host_version: String,
    /// Present only for the versioned platform-model path, and never stored in session history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "import(\"./model_context.ts\").AgentModelContext")]
    pub model_context: Option<crate::AgentModelContext>,
}

/// Wire protocol used inside the bidirectional `agent/acp` notification.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, TS, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "agent.ts")]
pub enum AgentProtocol {
    Acp,
}

/// Confirmation that a started agent is ready to receive ACP frames.
#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent.ts")]
pub struct AgentStartResult {
    pub protocol: AgentProtocol,
    pub acp_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "import(\"./model_context.ts\").AgentModelApplied")]
    pub model_applied: Option<crate::AgentModelApplied>,
}

impl AgentStartResult {
    /// Builds the only protocol result the current host accepts.
    pub fn acp_v1() -> Self {
        Self {
            protocol: AgentProtocol::Acp,
            acp_version: SUPPORTED_ACP_VERSION,
            model_applied: None,
        }
    }
}

/// One model an agent offers before any session exists.
#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent.ts")]
pub struct AgentModel {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub default: bool,
}

/// One command an agent reported before any session exists, as the agent worded it.
#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent.ts")]
pub struct AgentCommand {
    pub name: String,
    pub description: String,
    /// What to type after the command, when the agent said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub input_hint: Option<String>,
}

/// The Skills a session set up now would be given, in the values its setup would carry.
#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent.ts")]
pub struct AgentDiscoverSkills {
    /// The session's `additionalDirectories`, for an agent that declares support for them.
    #[ts(type = "Array<string>")]
    pub read_roots: Vec<PathBuf>,
    /// The session's `_meta` entry under `ora/skillDirectories`, absent there when empty.
    pub directories: Vec<String>,
    /// Differs after every change to the Skills, also one that leaves the other two alone.
    pub revision: String,
}

/// Supplies the workspace and the caller's needs for pre-session discovery.
#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent.ts")]
pub struct AgentDiscoverParams {
    #[ts(type = "string")]
    pub cwd: PathBuf,
    /// Whether the caller wants commands; a plugin must not wait for them when it does not.
    pub include_commands: bool,
    /// When the host stops waiting for the answer, in Unix milliseconds. Cleanup has to fit too.
    #[ts(type = "number")]
    pub deadline: i64,
    /// Present exactly when commands are wanted, because only they depend on Skills.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub skills: Option<AgentDiscoverSkills>,
}

/// What an agent offers before any session exists.
///
/// An absent command list and an empty one are different facts: absent means nobody knows, empty
/// means the agent said it has none.
#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent.ts")]
pub struct AgentDiscoverResult {
    pub models: Vec<AgentModel>,
    #[serde(
        default,
        deserialize_with = "well_formed_commands",
        skip_serializing_if = "Option::is_none"
    )]
    #[ts(optional)]
    pub commands: Option<Vec<AgentCommand>>,
}

/// Reads the reported commands, dropping each entry that is not one.
///
/// Commands are agent text a plugin relays, so a malformed entry must not cost the models that
/// travel with it. Anything that is not a list at all leaves the commands unknown.
fn well_formed_commands<'de, D>(deserializer: D) -> Result<Option<Vec<AgentCommand>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let serde_json::Value::Array(entries) = serde_json::Value::deserialize(deserializer)? else {
        return Ok(None);
    };
    Ok(Some(
        entries
            .into_iter()
            .filter_map(|entry| serde_json::from_value(entry).ok())
            .collect(),
    ))
}

/// Supplies the workspace context required by the models-only discovery method.
#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent.ts")]
pub struct AgentListModelsParams {
    #[ts(type = "string")]
    pub cwd: PathBuf,
}

/// Result of the models-only discovery method.
#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent.ts")]
pub struct AgentListModelsResult {
    pub models: Vec<AgentModel>,
}

/// Exports every agent control DTO into one TypeScript module.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    AgentStartContext::export(config)?;
    AgentProtocol::export(config)?;
    AgentStartResult::export(config)?;
    AgentModel::export(config)?;
    AgentCommand::export(config)?;
    AgentDiscoverSkills::export(config)?;
    AgentDiscoverParams::export(config)?;
    AgentDiscoverResult::export(config)?;
    AgentListModelsParams::export(config)?;
    AgentListModelsResult::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        AgentCommand, AgentDiscoverParams, AgentDiscoverResult, AgentDiscoverSkills,
        AgentListModelsParams, AgentModel,
    };
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::path::PathBuf;

    /// Discovery params travel as one camelCase `cwd` string, the shape `agent/start` already uses.
    ///
    /// Pinned here rather than left to the derive because this is the wire an installed plugin
    /// built against a published SDK reads: a rename or a path encoding change would leave older
    /// plugins silently discovering models against the wrong directory rather than failing.
    #[test]
    fn discovery_params_carry_the_workspace_directory_as_cwd() {
        assert_eq!(
            serde_json::to_value(AgentListModelsParams {
                cwd: PathBuf::from("/projects/ora"),
            })
            .expect("serialize discovery params"),
            json!({ "cwd": "/projects/ora" }),
        );
    }

    /// The wire plugins built against a published SDK read, pinned for the same reason.
    #[test]
    fn discover_params_say_whether_commands_are_wanted() {
        let params = |skills: Option<AgentDiscoverSkills>| AgentDiscoverParams {
            cwd: PathBuf::from("/projects/ora"),
            include_commands: skills.is_some(),
            deadline: 1_700_000_060_000,
            skills,
        };
        assert_eq!(
            serde_json::to_value(params(Some(AgentDiscoverSkills {
                read_roots: vec![PathBuf::from("/data/skills")],
                directories: vec!["/data/skills/review".to_string()],
                revision: "7".to_string(),
            })))
            .expect("serialize discovery params"),
            json!({
                "cwd": "/projects/ora",
                "includeCommands": true,
                "deadline": 1_700_000_060_000_i64,
                "skills": {
                    "readRoots": ["/data/skills"],
                    "directories": ["/data/skills/review"],
                    "revision": "7",
                },
            }),
        );
        assert_eq!(
            serde_json::to_value(params(None)).expect("serialize discovery params"),
            json!({
                "cwd": "/projects/ora",
                "includeCommands": false,
                "deadline": 1_700_000_060_000_i64,
            }),
        );
    }

    /// Not knowing the commands and knowing there are none stay apart across the wire.
    #[test]
    fn unknown_commands_stay_distinct_from_an_empty_list() {
        for (wire, commands) in [
            (json!({ "models": [] }), None),
            (json!({ "models": [], "commands": null }), None),
            (json!({ "models": [], "commands": [] }), Some(Vec::new())),
            // Not a list at all says nothing about the commands, and costs no models.
            (json!({ "models": [], "commands": "review" }), None),
            (json!({ "models": [], "commands": {} }), None),
        ] {
            let result: AgentDiscoverResult = serde_json::from_value(wire).expect("deserialize");
            assert_eq!(result.commands, commands);
        }
        assert_eq!(
            serde_json::to_value(AgentDiscoverResult {
                models: Vec::new(),
                commands: None,
            })
            .expect("serialize"),
            json!({ "models": [] }),
        );
    }

    /// A malformed command is dropped alone; the models and the other commands are kept in order.
    #[test]
    fn malformed_commands_are_dropped_one_by_one() {
        let result: AgentDiscoverResult = serde_json::from_value(json!({
            "models": [{ "id": "m1", "displayName": "Model 1" }],
            "commands": [
                { "name": "review", "description": "Review changes", "inputHint": "scope" },
                { "name": 7, "description": "Not a name" },
                "plan",
                { "name": "init", "description": "Set up", "extra": true },
            ],
        }))
        .expect("deserialize");

        assert_eq!(
            result,
            AgentDiscoverResult {
                models: vec![AgentModel {
                    id: "m1".to_string(),
                    display_name: "Model 1".to_string(),
                    default: false,
                }],
                commands: Some(vec![
                    AgentCommand {
                        name: "review".to_string(),
                        description: "Review changes".to_string(),
                        input_hint: Some("scope".to_string()),
                    },
                    AgentCommand {
                        name: "init".to_string(),
                        description: "Set up".to_string(),
                        input_hint: None,
                    },
                ]),
            },
        );
    }

    /// A plugin answering an older host still parses today's params.
    #[test]
    fn discovery_params_round_trip() {
        let params = AgentListModelsParams {
            cwd: PathBuf::from("/projects/ora"),
        };
        assert_eq!(
            serde_json::from_value::<AgentListModelsParams>(
                serde_json::to_value(&params).expect("serialize")
            )
            .expect("deserialize"),
            params,
        );
    }
}
