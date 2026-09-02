use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use ts_rs::TS;

/// Method that starts the agent process owned by a plugin.
pub const AGENT_START_METHOD: &str = "agent/start";
/// Method that stops the agent while leaving its plugin process alive.
pub const AGENT_STOP_METHOD: &str = "agent/stop";
/// Method that lists models before an ACP session exists.
pub const AGENT_LIST_MODELS_METHOD: &str = "agent/list_models";
/// Bidirectional notification that carries one opaque ACP frame.
pub const AGENT_ACP_METHOD: &str = "agent/acp";

/// Error returned when the agent executable is not installed on the machine.
pub const AGENT_NOT_INSTALLED_CODE: i64 = -32001;
/// Error returned when the executable bundled by an agent package cannot run.
pub const AGENT_UNUSABLE_CODE: i64 = -32002;
/// ACP major version carried over the agent plugin channel.
pub const SUPPORTED_ACP_VERSION: u32 = 1;

/// Host context handed to an agent when its underlying process starts.
#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent.ts")]
pub struct AgentStartContext {
    #[ts(type = "string")]
    pub cwd: PathBuf,
    pub host_version: String,
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
}

impl AgentStartResult {
    /// Builds the only protocol result the current host accepts.
    pub fn acp_v1() -> Self {
        Self {
            protocol: AgentProtocol::Acp,
            acp_version: SUPPORTED_ACP_VERSION,
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

/// Supplies the workspace context required for pre-session model discovery.
#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent.ts")]
pub struct AgentListModelsParams {
    #[ts(type = "string")]
    pub cwd: PathBuf,
}

/// Result of the agent model discovery method.
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
    AgentListModelsParams::export(config)?;
    AgentListModelsResult::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::AgentListModelsParams;
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
