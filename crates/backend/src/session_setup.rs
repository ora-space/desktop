//! Unified Session Setup boundary shared by every ACP `session/new` and `session/load`.
//!
//! The first field is MCP. Future session inputs such as hooks would become additional typed
//! fields here rather than an open JSON map or a private Host/Agent method.

mod barrier;
mod mcp;

pub(crate) use barrier::{AgentSessionBarriers, BarrierGuard, BarrierReason};
pub(crate) use mcp::{
    AgentSessionMcpCapabilities, LiveMcpEvent, LiveMcpPromptAdmission, LiveMcpState,
    SessionMcpError, SessionMcpHost, SessionMcpRevision, SessionMcpSnapshot, resolve_session_mcp,
    resolve_session_mcp_revision,
};

use crate::plugin::PluginApi;
use std::path::Path;
use std::sync::Arc;

/// Resolved inputs that every ACP session setup or refresh must send together.
#[derive(Clone)]
pub(crate) struct SessionSetup {
    pub mcp: SessionMcpSnapshot,
}

impl SessionSetup {
    /// Resolves the current Effective MCP Set for one Agent, one cwd, and one capability snapshot.
    pub(crate) fn resolve(
        host: &SessionMcpHost,
        cwd: &Path,
        capabilities: AgentSessionMcpCapabilities,
    ) -> Result<Self, SessionMcpError> {
        Ok(Self {
            mcp: resolve_session_mcp(host, host, cwd, capabilities)?,
        })
    }
}

impl SessionMcpHost {
    /// Builds the host-backed catalog and configuration source used by every Session setup path.
    pub(crate) fn from_plugin_api(plugin_host: Arc<PluginApi>) -> Self {
        Self::new(plugin_host)
    }
}
