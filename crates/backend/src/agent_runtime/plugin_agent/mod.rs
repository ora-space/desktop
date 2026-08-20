mod control;
mod inbound;
mod transport;

#[cfg(test)]
mod tests;

pub(crate) use control::{PluginAgentError, PluginAgentModel, list_models, stop_agent};
pub(crate) use transport::{AgentTransport, PluginAcpTransport};

use std::path::{Path, PathBuf};
use std::time::Duration;

use ora_acp::AcpMessages;
use ora_plugin_lifecycle::agent_permissions;
use ora_plugin_runtime::{PluginRuntime, PluginRuntimeConfig};
use ora_process::TokioProcessSpawner;

use crate::bootstrap::AgentPluginPackage;

/// How long a plugin has to publish its capability registration before it is considered dead.
const PLUGIN_READY_TIMEOUT: Duration = Duration::from_secs(10);
/// How long one control method may run. ACP traffic is a notification and is not bounded by this.
const PLUGIN_CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a plugin has to exit after `ora/shutdown` before its process tree is killed.
const PLUGIN_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

/// Describes one installed agent plugin the connection supervisor can launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PluginAgentSpec {
    /// The plugin package id, which is also this agent's identity throughout the host.
    pub plugin_id: String,
    pub deno_path: PathBuf,
    pub entrypoint: PathBuf,
}

impl From<AgentPluginPackage> for PluginAgentSpec {
    fn from(package: AgentPluginPackage) -> Self {
        Self {
            plugin_id: package.id,
            deno_path: package.deno_path,
            entrypoint: package.entrypoint,
        }
    }
}

/// Holds one running agent plugin together with the ACP stream it feeds.
pub(crate) struct LaunchedPluginAgent {
    pub runtime: PluginRuntime,
    pub messages: AcpMessages,
}

/// Starts one agent plugin and brings up the agent behind it.
///
/// On return the plugin has registered a complete agent contract and confirmed its agent is ready
/// to receive ACP frames, so the caller can immediately begin the ACP handshake.
pub(crate) async fn launch(
    spec: &PluginAgentSpec,
    home_directory: &Path,
    host_version: &str,
) -> Result<LaunchedPluginAgent, PluginAgentError> {
    // Permissions come from the lifecycle crate so both launch paths grant the same set; the agent
    // set has no path grants, so rendering cannot fail and the error branch is unreachable.
    let permissions = agent_permissions()
        .iter()
        .map(|permission| {
            permission
                .to_flag()
                .map(|flag| flag.to_string_lossy().into_owned())
                .map_err(|error| PluginAgentError::Failed(error.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let config = PluginRuntimeConfig {
        plugin_id: spec.plugin_id.clone(),
        deno_path: spec.deno_path.clone(),
        entrypoint: spec.entrypoint.clone(),
        permissions,
        cwd: None,
        env: Vec::new(),
        ready_timeout: PLUGIN_READY_TIMEOUT,
        call_timeout: PLUGIN_CALL_TIMEOUT,
        shutdown_timeout: PLUGIN_SHUTDOWN_TIMEOUT,
    };
    let (runtime, mut notifications) =
        PluginRuntime::launch(&TokioProcessSpawner::new(), config).await?;
    if let Err(error) = control::verify_agent_contract(&runtime.registration().await) {
        runtime.shutdown_and_wait().await;
        return Err(error);
    }
    if let Err(error) = control::start_agent(&runtime, home_directory, host_version).await {
        runtime.shutdown_and_wait().await;
        return Err(error);
    }
    inbound::discard_frames_before_start(&mut notifications, &spec.plugin_id);

    Ok(LaunchedPluginAgent {
        runtime,
        messages: inbound::spawn_frame_forwarding(notifications, spec.plugin_id.clone()),
    })
}
