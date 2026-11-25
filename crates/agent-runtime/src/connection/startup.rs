//! Bringing one agent plugin up to an initialized ACP connection, and classifying why it failed.
//!
//! Separate from the supervisor loop because this is where a startup failure is judged worth
//! retrying or not; the loop only acts on that judgement.

use super::AgentAcpClient;
use crate::RuntimeError;
use crate::host::AgentAttach;
use crate::plugin_agent::{self, LaunchedPluginAgent, PluginAcpTransport, PluginAgentError};
use crate::suspend::{AgentProcess, stop_plugin_runtime};
use crate::{
    INITIALIZE_TIMEOUT, agent_not_installed, agent_start_failed, agent_timed_out, map_acp_error,
};
use agent_client_protocol_schema::ProtocolVersion;
use agent_client_protocol_schema::v1::AGENT_METHOD_NAMES;
use agent_client_protocol_schema::v1::{
    ClientCapabilities, ClientSessionCapabilities, Implementation, InitializeRequest,
    InitializeResponse, SessionConfigOptionsCapabilities,
};
use ora_acp::{AcpInboundEvent, AcpMessages, AcpPeer};
use ora_domain::PluginId;
use ora_plugin_lifecycle::ConnectionError;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::timeout;

/// Separates a startup failure worth retrying from one that can never succeed.
///
/// Almost every failure is retryable: an agent can be installed later, a crashed provider can come
/// back. A provider that does not implement the contract this host requires is different — it will
/// fail identically forever, so retrying only produces a warning every backoff interval and never
/// a working agent.
pub(super) enum StartFailure {
    Retryable(RuntimeError),
    Terminal(RuntimeError),
}

impl From<RuntimeError> for StartFailure {
    fn from(error: RuntimeError) -> Self {
        Self::Retryable(error)
    }
}

/// Holds everything one agent source produces before the ACP handshake runs.
struct StartedAgent<A: AgentAttach> {
    process: AgentProcess<A>,
    transport: PluginAcpTransport,
    messages: AcpMessages,
}

/// One initialized agent process with the capabilities its ACP handshake reported.
pub(super) struct SharedProcess<A: AgentAttach> {
    pub process: AgentProcess<A>,
    pub client: AgentAcpClient,
    pub inbound: mpsc::UnboundedReceiver<AcpInboundEvent>,
    pub load_session_supported: bool,
    pub http_mcp_supported: bool,
    pub list_session_supported: bool,
    pub close_session_supported: bool,
    pub delete_session_supported: bool,
    pub additional_directories_supported: bool,
}

/// Starts one agent in the neutral home directory and completes the ACP handshake.
///
/// The connection is only reported ready once ACP `initialize` has returned its capabilities, so
/// no caller can send a session request to a transport that is not yet carrying a live agent.
pub(super) async fn spawn_initialized_process<A: AgentAttach>(
    plugin_id: &PluginId,
    plugin_host: &Arc<A>,
    home_directory: &Path,
) -> Result<SharedProcess<A>, StartFailure> {
    let StartedAgent {
        process,
        transport,
        messages,
    } = spawn_plugin_connection(plugin_id, plugin_host, home_directory).await?;
    let peer = AcpPeer::spawn(messages, transport);
    // Config options are only sent by agents that see the client advertise them,
    // so the model selector depends on this declaration. Boolean options stay
    // undeclared because Ora renders only select-style options today; claiming
    // support would invite payloads the client silently drops.
    let initialize = InitializeRequest::new(ProtocolVersion::V1)
        .client_capabilities(
            ClientCapabilities::new().session(
                ClientSessionCapabilities::new()
                    .config_options(SessionConfigOptionsCapabilities::new()),
            ),
        )
        .client_info(Implementation::new("ora", env!("CARGO_PKG_VERSION")));
    let response = match timeout(
        INITIALIZE_TIMEOUT,
        peer.client
            .request::<_, InitializeResponse>(AGENT_METHOD_NAMES.initialize, &initialize),
    )
    .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            process.terminate_and_reap().await;
            return Err(StartFailure::Retryable(map_acp_error(error)));
        }
        Err(_) => {
            process.terminate_and_reap().await;
            return Err(StartFailure::Retryable(agent_timed_out(
                "agent initialization timed out",
            )));
        }
    };
    let (client, inbound) = peer.into_parts();
    Ok(SharedProcess {
        process,
        client,
        inbound,
        load_session_supported: response.agent_capabilities.load_session,
        http_mcp_supported: response.agent_capabilities.mcp_capabilities.http,
        list_session_supported: response
            .agent_capabilities
            .session_capabilities
            .list
            .is_some(),
        close_session_supported: response
            .agent_capabilities
            .session_capabilities
            .close
            .is_some(),
        delete_session_supported: response
            .agent_capabilities
            .session_capabilities
            .delete
            .is_some(),
        additional_directories_supported: response
            .agent_capabilities
            .session_capabilities
            .additional_directories
            .is_some(),
    })
}

/// Attaches to one lifecycle-owned agent plugin and wires ACP over its notification channel.
async fn spawn_plugin_connection<A: AgentAttach>(
    plugin_id: &PluginId,
    plugin_host: &Arc<A>,
    home_directory: &Path,
) -> Result<StartedAgent<A>, StartFailure> {
    let attachment = plugin_host
        .attach_agent(plugin_id)
        .await
        .map_err(plugin_attach_error)?;
    let LaunchedPluginAgent { runtime, messages } = match plugin_agent::attach(
        attachment,
        &plugin_id.canonical(),
        home_directory,
        env!("CARGO_PKG_VERSION"),
        plugin_host.model_context(),
    )
    .await
    {
        Ok(launched) => launched,
        Err(error) => {
            stop_plugin_runtime(plugin_host.as_ref(), plugin_id).await;
            return Err(plugin_start_error(error));
        }
    };
    let transport = PluginAcpTransport::new(runtime.clone());
    Ok(StartedAgent {
        process: AgentProcess {
            plugin_id: plugin_id.clone(),
            runtime,
            host: plugin_host.clone(),
        },
        transport,
        messages,
    })
}

/// Maps a lifecycle refusal to start a plugin onto the supervisor's retry classification.
///
/// An uninstalled plugin is reported like a missing CLI so the supervisor retries without noisy
/// logging while package discovery catches up.
pub(super) fn plugin_attach_error(error: ConnectionError) -> StartFailure {
    match error {
        ConnectionError::NotFound | ConnectionError::NoProcess => StartFailure::Retryable(
            agent_not_installed("the plugin behind this agent is not available"),
        ),
        ConnectionError::Timeout => {
            StartFailure::Retryable(agent_timed_out("agent plugin start timed out"))
        }
        ConnectionError::Failed(_) | ConnectionError::NotReady | ConnectionError::NotRunning => {
            StartFailure::Retryable(agent_start_failed(error.to_string()))
        }
    }
}

/// Maps a plugin startup failure onto the supervisor's retry classification.
///
/// A plugin whose own agent process is not installed on this machine is an expected local
/// configuration, so it is reported exactly like an uninstalled plugin and retried without
/// logging: the user can install the CLI while Ora keeps running, and the next attempt picks it
/// up. A plugin that reports its own bundled agent as unusable is the opposite case — the same
/// package produces the same failure on every attempt — so it is abandoned like an unservable
/// contract rather than retried behind a quiet `agent_not_installed`.
pub(super) fn plugin_start_error(error: PluginAgentError) -> StartFailure {
    match error {
        PluginAgentError::AgentNotInstalled => StartFailure::Retryable(agent_not_installed(
            "the agent behind this plugin is not installed",
        )),
        PluginAgentError::AgentUnusable(detail) => {
            StartFailure::Terminal(agent_start_failed(detail))
        }
        PluginAgentError::ContractIncomplete(detail) => {
            StartFailure::Terminal(agent_start_failed(detail))
        }
        PluginAgentError::Failed(detail) => StartFailure::Retryable(agent_start_failed(detail)),
    }
}
