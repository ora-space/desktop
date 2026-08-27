use std::io;

use ora_acp::{AcpError, AcpTransport};
use ora_plugin_runtime::PluginRuntime;
use serde_json::Value;

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
