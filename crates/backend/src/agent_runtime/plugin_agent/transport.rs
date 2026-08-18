use std::io;

use ora_acp::{AcpError, AcpTransport, NdjsonTransport};
use ora_plugin_runtime::PluginRuntime;
use serde_json::Value;
use tokio::process::ChildStdin;

use super::control::AGENT_ACP_METHOD;

/// Relays whole ACP messages to one agent plugin as `agent/acp` notifications.
///
/// The host never inspects the payload. A notification is used rather than a plugin method call
/// because ACP already carries its own ids, cancellation, and ordering; layering the runtime's
/// request correlation on top would mean two timeouts and two cancellation paths for one frame,
/// and would bound multi-minute prompts by a control-call timeout.
pub(crate) struct PluginAcpTransport {
    runtime: PluginRuntime,
}

impl PluginAcpTransport {
    pub(crate) fn new(runtime: PluginRuntime) -> Self {
        Self { runtime }
    }
}

impl AcpTransport for PluginAcpTransport {
    async fn send(&self, message: Value) -> Result<(), AcpError> {
        self.runtime
            .notify(AGENT_ACP_METHOD, message)
            .await
            .map_err(|error| AcpError::Io(io::Error::other(error.to_string())))
    }
}

/// Selects the transport that carries one connection's ACP traffic.
///
/// `RuntimeConnection` is published through a `watch` channel, so the transport cannot remain a
/// generic parameter of the connection type. An enum keeps dispatch static and every match
/// exhaustive, which a trait object would give up.
///
/// The `Stdio` variant exists only while Ora still launches agent CLIs itself; once every builtin
/// CLI ships as a plugin, plugins are the sole transport and this enum collapses.
pub(crate) enum AgentTransport {
    Stdio(NdjsonTransport<ChildStdin>),
    Plugin(PluginAcpTransport),
}

impl AcpTransport for AgentTransport {
    async fn send(&self, message: Value) -> Result<(), AcpError> {
        match self {
            Self::Stdio(transport) => transport.send(message).await,
            Self::Plugin(transport) => transport.send(message).await,
        }
    }
}
