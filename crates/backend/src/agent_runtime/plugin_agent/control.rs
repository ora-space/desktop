use std::path::Path;
use std::time::Duration;

use ora_logging::ora_warn;
use ora_plugin_runtime::PluginEffectCoordination;
use ora_plugin_runtime::{PluginRegistration, PluginRuntime, PluginRuntimeError};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;
use tokio::time::timeout;

/// The method a plugin must serve to bring its agent up.
pub(super) const AGENT_START_METHOD: &str = "agent/start";
/// The method a plugin must serve to bring its agent down while staying alive itself.
pub(super) const AGENT_STOP_METHOD: &str = "agent/stop";
/// The method a plugin must serve to list selectable models outside any session.
pub(super) const AGENT_LIST_MODELS_METHOD: &str = "agent/listModels";
/// The notification method that carries ACP frames in both directions.
pub(crate) const AGENT_ACP_METHOD: &str = "agent/acp";

/// How long a plugin has to confirm it stopped its agent.
///
/// This is far shorter than the general control-call timeout, and shorter than the host's own
/// cancellation grace, because teardown must never be the reason shutdown stalls: the plugin
/// process is ended immediately afterwards whether or not it answered.
const AGENT_STOP_TIMEOUT: Duration = Duration::from_secs(2);

/// The error code a plugin returns when the agent it fronts is not installed on this machine.
///
/// This is an expected local configuration rather than a fault, so the supervisor retries it
/// silently; every other code is a genuine startup failure worth logging.
const AGENT_NOT_INSTALLED_CODE: i64 = -32001;

/// Reports why one agent plugin could not be brought up or queried.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub(crate) enum PluginAgentError {
    #[error("agent plugin contract is incomplete: {0}")]
    ContractIncomplete(String),
    #[error("the plugin's agent is not installed")]
    AgentNotInstalled,
    #[error("agent plugin failed: {0}")]
    Failed(String),
}

impl From<PluginRuntimeError> for PluginAgentError {
    fn from(error: PluginRuntimeError) -> Self {
        match error {
            PluginRuntimeError::Remote {
                code: AGENT_NOT_INSTALLED_CODE,
                ..
            } => Self::AgentNotInstalled,
            other => Self::Failed(other.to_string()),
        }
    }
}

/// Parameters handed to a plugin when the host asks it to bring its agent up.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentStartParams<'a> {
    cwd: &'a Path,
    host_version: &'a str,
}

/// The plugin's confirmation that its agent is ready to receive ACP frames.
#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct AgentStartResult {
    protocol: AgentProtocol,
    acp_version: u32,
}

/// The wire protocol a plugin speaks on the ACP channel.
///
/// Only ACP exists today; the field is on the wire so a future translating plugin can declare a
/// different protocol without changing the notification channel's shape.
#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum AgentProtocol {
    Acp,
}

/// The ACP major version this host speaks over the plugin channel.
const SUPPORTED_ACP_VERSION: u32 = 1;

/// Describes one model an agent plugin offers before any session exists.
///
/// This is deliberately separate from ACP session config options, which only exist after
/// `session/new`: the agent and model pickers must render before any session is created.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginAgentModel {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub default: bool,
}

/// Carries the model list one plugin advertises.
#[derive(Debug, Deserialize, PartialEq, Eq)]
struct ListModelsResult {
    models: Vec<PluginAgentModel>,
}

/// Rejects a plugin whose registration does not cover the whole agent contract.
///
/// This runs the moment the handshake completes, before any session exists and before a user is
/// waiting on a prompt, so a contract mismatch surfaces as an unavailable agent instead of a
/// failure in the middle of someone's turn.
pub(super) fn verify_agent_contract(
    registration: &PluginRegistration,
) -> Result<(), PluginAgentError> {
    let missing_methods = [
        AGENT_START_METHOD,
        AGENT_STOP_METHOD,
        AGENT_LIST_MODELS_METHOD,
    ]
    .into_iter()
    .filter(|method| !registration.methods.contains(*method))
    .collect::<Vec<_>>();
    if !missing_methods.is_empty() {
        return Err(PluginAgentError::ContractIncomplete(format!(
            "missing methods {}",
            missing_methods.join(", ")
        )));
    }
    if !registration.emits.contains(AGENT_ACP_METHOD) {
        return Err(PluginAgentError::ContractIncomplete(format!(
            "missing emitted method {AGENT_ACP_METHOD}"
        )));
    }
    let requires_restart_coordination = registration
        .effect_surfaces
        .iter()
        .any(|surface| surface.coordination == PluginEffectCoordination::WaitForIdleAndRestart);
    if requires_restart_coordination {
        let missing_effect_methods = [
            super::effect::WAIT_FOR_IDLE_METHOD,
            super::effect::RESTART_METHOD,
        ]
        .into_iter()
        .filter(|method| !registration.methods.contains(*method))
        .collect::<Vec<_>>();
        if !missing_effect_methods.is_empty() {
            return Err(PluginAgentError::ContractIncomplete(format!(
                "missing Effect methods {}",
                missing_effect_methods.join(", ")
            )));
        }
    }
    Ok(())
}

/// Brings one plugin's agent up and confirms it will speak a protocol this host understands.
pub(super) async fn start_agent(
    runtime: &PluginRuntime,
    cwd: &Path,
    host_version: &str,
) -> Result<(), PluginAgentError> {
    let params = serde_json::to_value(AgentStartParams { cwd, host_version })
        .map_err(|error| PluginAgentError::Failed(error.to_string()))?;
    let result = runtime.invoke(AGENT_START_METHOD, params).await?;
    let result: AgentStartResult = serde_json::from_value(result).map_err(|error| {
        PluginAgentError::Failed(format!("invalid {AGENT_START_METHOD} result: {error}"))
    })?;
    if result.acp_version != SUPPORTED_ACP_VERSION {
        return Err(PluginAgentError::ContractIncomplete(format!(
            "unsupported ACP version {}; expected {SUPPORTED_ACP_VERSION}",
            result.acp_version
        )));
    }
    let AgentProtocol::Acp = result.protocol;
    Ok(())
}

/// Asks one plugin to terminate the agent it owns while leaving the plugin process alive.
///
/// Failures are logged rather than propagated: plugin cleanup is best effort, and the host always
/// reaps the process tree afterwards regardless of what the plugin did. Dropping the plugin
/// runtime is what ends the plugin itself, so stopping the agent first is the plugin's one chance
/// to reap the CLI it owns.
pub(crate) async fn stop_agent(runtime: &PluginRuntime, plugin_id: &str) {
    match timeout(
        AGENT_STOP_TIMEOUT,
        runtime.invoke(AGENT_STOP_METHOD, json!({})),
    )
    .await
    {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => ora_warn!(
            plugin_id = %plugin_id,
            error = %error,
            "agent plugin did not confirm shutdown"
        ),
        Err(_) => ora_warn!(
            plugin_id = %plugin_id,
            "agent plugin did not answer shutdown before its deadline"
        ),
    }
}

/// Reads the models one plugin offers for the agent and model pickers.
pub(crate) async fn list_models(
    runtime: &PluginRuntime,
) -> Result<Vec<PluginAgentModel>, PluginAgentError> {
    let result = runtime.invoke(AGENT_LIST_MODELS_METHOD, json!({})).await?;
    let result: ListModelsResult = serde_json::from_value(result).map_err(|error| {
        PluginAgentError::Failed(format!(
            "invalid {AGENT_LIST_MODELS_METHOD} result: {error}"
        ))
    })?;
    Ok(result.models)
}
