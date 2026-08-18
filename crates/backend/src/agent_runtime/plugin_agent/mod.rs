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
use ora_plugin_runtime::{PluginRuntime, PluginRuntimeConfig};
use ora_process::TokioProcessSpawner;

use crate::bootstrap::AgentPluginPackage;

/// How long a plugin has to publish its capability registration before it is considered dead.
const PLUGIN_READY_TIMEOUT: Duration = Duration::from_secs(10);
/// How long one control method may run. ACP traffic is a notification and is not bounded by this.
const PLUGIN_CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a plugin has to exit after `ora/shutdown` before its process tree is killed.
const PLUGIN_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

/// The Deno permissions granted to an agent plugin.
///
/// An agent plugin spawns and owns the agent CLI itself, so it needs `--allow-run` and everything
/// that CLI needs to work. That makes an agent plugin roughly as privileged as the host. This is a
/// deliberate, documented gap: capability narrowing for agent plugins is deferred until the agent
/// contract itself is proven, and closing it later changes only how the agent is started, never
/// the `agent/acp` pipe.
const AGENT_PLUGIN_PERMISSIONS: [&str; 4] =
    ["--allow-run", "--allow-read", "--allow-env", "--allow-net"];

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
    let config = PluginRuntimeConfig {
        plugin_id: spec.plugin_id.clone(),
        deno_path: spec.deno_path.clone(),
        entrypoint: spec.entrypoint.clone(),
        permissions: AGENT_PLUGIN_PERMISSIONS.map(str::to_string).to_vec(),
        ready_timeout: PLUGIN_READY_TIMEOUT,
        call_timeout: PLUGIN_CALL_TIMEOUT,
        shutdown_timeout: PLUGIN_SHUTDOWN_TIMEOUT,
    };
    let (runtime, mut notifications) =
        PluginRuntime::launch(&TokioProcessSpawner::new(), config).await?;
    control::verify_agent_contract(&runtime.registration().await)?;
    control::start_agent(&runtime, home_directory, host_version).await?;
    inbound::discard_frames_before_start(&mut notifications, &spec.plugin_id);

    Ok(LaunchedPluginAgent {
        runtime,
        messages: inbound::spawn_frame_forwarding(notifications, spec.plugin_id.clone()),
    })
}
