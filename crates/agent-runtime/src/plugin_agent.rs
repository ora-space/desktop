mod control;
mod discovery;
mod inbound;
mod transport;

#[cfg(test)]
mod tests;

pub(crate) use control::{PluginAgentError, stop_agent};
pub(crate) use discovery::{DiscoveryScope, discover};
pub(crate) use transport::PluginAcpTransport;

use std::path::Path;

use ora_acp::AcpMessages;
use ora_plugin_runtime::PluginRuntime;

use crate::host::AgentPluginAttachment;

/// Holds one running agent plugin together with the ACP stream it feeds.
pub(crate) struct LaunchedPluginAgent {
    pub runtime: PluginRuntime,
    pub messages: AcpMessages,
}

/// Brings up the agent behind one already-running plugin process.
///
/// The process is owned by the plugin lifecycle, so this only speaks the agent contract over it:
/// enabling, stopping, and uninstalling keep deciding how long the plugin lives, while a
/// connection owns nothing beyond the notification tap of the generation it attached to.
///
/// On return the plugin has published a complete agent registration and confirmed its agent is
/// ready to receive ACP frames, so the caller can immediately begin the ACP handshake. A failure
/// leaves the process to the caller, which asks the lifecycle to stop it.
pub(crate) async fn attach(
    attachment: AgentPluginAttachment,
    plugin_id: &str,
    home_directory: &Path,
    host_version: &str,
    model_context: Option<ora_plugin_protocol::AgentModelContext>,
) -> Result<LaunchedPluginAgent, PluginAgentError> {
    let AgentPluginAttachment {
        runtime,
        mut notifications,
    } = attachment;
    let registration = runtime.registration().await;
    control::verify_agent_contract(&registration)?;
    let plugin_id = ora_domain::PluginId::parse(plugin_id).map_err(|error| {
        PluginAgentError::ContractIncomplete(format!("invalid plugin identity: {error}"))
    })?;
    control::start_agent(&runtime, home_directory, host_version, model_context).await?;
    inbound::discard_frames_before_start(&mut notifications, &plugin_id.canonical());

    Ok(LaunchedPluginAgent {
        runtime,
        messages: inbound::spawn_frame_forwarding(notifications, plugin_id.to_string()),
    })
}
