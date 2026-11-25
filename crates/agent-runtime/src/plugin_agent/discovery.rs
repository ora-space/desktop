//! Asks one agent plugin what its agent offers before any session exists.

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use ora_plugin_protocol::{
    AGENT_DISCOVER_METHOD, AGENT_DISCOVERY_BUDGET_MS, AGENT_LIST_MODELS_METHOD,
    AgentDiscoverParams, AgentDiscoverResult, AgentListModelsParams, AgentListModelsResult,
};
use ora_plugin_runtime::PluginRuntime;
use serde::Serialize;
use serde::de::DeserializeOwned;

use tokio::time::Instant;

use super::control::PluginAgentError;
use crate::clock::SystemClock;

/// One moment at which a discovery ends, on the clock each side measures it by.
#[derive(Clone, Copy)]
struct Deadline {
    /// What the host waits until.
    at: Instant,
    /// What the plugin is told.
    unix_millis: i64,
}

impl Deadline {
    /// Discovery may start a one-shot agent process and therefore has a dedicated budget.
    fn from_now() -> Self {
        Self {
            at: Instant::now() + Duration::from_millis(AGENT_DISCOVERY_BUDGET_MS),
            unix_millis: SystemClock.now_timestamp_millis() + AGENT_DISCOVERY_BUDGET_MS as i64,
        }
    }
}

/// Which facts a discovery is made for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiscoveryScope {
    /// Never waits for an agent to report its commands.
    Models,
    ModelsAndCommands,
}

/// The one method a discovery calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DiscoveryMethod {
    Discover,
    /// Plugins published before `agent/discover`, which know no commands.
    ListModels,
}

/// Picks the method from what the plugin registered.
pub(super) fn discovery_method(methods: &HashSet<String>) -> DiscoveryMethod {
    if methods.contains(AGENT_DISCOVER_METHOD) {
        DiscoveryMethod::Discover
    } else {
        DiscoveryMethod::ListModels
    }
}

/// Runs one discovery against the plugin.
///
/// Exactly one method is called, also when it fails: the models-only method is the same discovery
/// under its old name, so falling back to it would only start the agent a second time.
pub(crate) async fn discover(
    runtime: &PluginRuntime,
    cwd: &Path,
    scope: DiscoveryScope,
) -> Result<AgentDiscoverResult, PluginAgentError> {
    let cwd = cwd.to_path_buf();
    let deadline = Deadline::from_now();
    match discovery_method(&runtime.registration().await.methods) {
        DiscoveryMethod::Discover => {
            let include_commands = scope == DiscoveryScope::ModelsAndCommands;
            let params = AgentDiscoverParams {
                cwd,
                include_commands,
                deadline: deadline.unix_millis,
            };
            let mut result: AgentDiscoverResult =
                call(runtime, AGENT_DISCOVER_METHOD, params, deadline).await?;
            if !include_commands {
                result.commands = None;
            }
            Ok(result)
        }
        DiscoveryMethod::ListModels => {
            let result: AgentListModelsResult = call(
                runtime,
                AGENT_LIST_MODELS_METHOD,
                AgentListModelsParams { cwd },
                deadline,
            )
            .await?;
            Ok(AgentDiscoverResult {
                models: result.models,
                commands: None,
            })
        }
    }
}

/// Invokes one discovery method, waiting no longer than the deadline.
async fn call<Params: Serialize, Response: DeserializeOwned>(
    runtime: &PluginRuntime,
    method: &str,
    params: Params,
    deadline: Deadline,
) -> Result<Response, PluginAgentError> {
    let params = serde_json::to_value(params)
        .map_err(|error| PluginAgentError::Failed(error.to_string()))?;
    let result = runtime.invoke_until(method, params, deadline.at).await?;
    serde_json::from_value(result)
        .map_err(|error| PluginAgentError::Failed(format!("invalid {method} result: {error}")))
}
