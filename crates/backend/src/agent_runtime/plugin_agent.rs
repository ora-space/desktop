mod control;
mod effect;
mod inbound;
mod transport;

#[cfg(test)]
mod tests;

pub(crate) use control::{PluginAgentError, list_models, stop_agent};
pub(crate) use effect::{coordinate, reactivate, verify_ready};
pub(crate) use transport::PluginAcpTransport;

use std::path::Path;

use ora_acp::AcpMessages;
use ora_effect::ConsumerDeclaration;
use ora_plugin_runtime::PluginRuntime;

use crate::plugin::AgentPluginAttachment;

/// Holds one running agent plugin together with the ACP stream it feeds.
pub(crate) struct LaunchedPluginAgent {
    pub runtime: PluginRuntime,
    pub messages: AcpMessages,
    pub effect_declaration: Option<ConsumerDeclaration>,
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
) -> Result<LaunchedPluginAgent, PluginAgentError> {
    let AgentPluginAttachment {
        connection,
        mut notifications,
    } = attachment;
    let runtime = connection.runtime().process().clone();
    let registration = runtime.registration().await;
    control::verify_agent_contract(&registration)?;
    let plugin_id = ora_domain::PluginId::parse(plugin_id).map_err(|error| {
        PluginAgentError::ContractIncomplete(format!("invalid plugin identity: {error}"))
    })?;
    let effect_declaration = effect::registered_consumer_declaration(&plugin_id, &registration)
        .map_err(|error| PluginAgentError::ContractIncomplete(error.to_string()))?;
    control::start_agent(&runtime, home_directory, host_version).await?;
    inbound::discard_frames_before_start(&mut notifications, &plugin_id.canonical());

    Ok(LaunchedPluginAgent {
        runtime,
        messages: inbound::spawn_frame_forwarding(notifications, plugin_id.to_string()),
        effect_declaration,
    })
}
