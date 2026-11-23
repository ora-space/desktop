//! Desktop's Session Setup source for every ACP `session/new` and `session/load`.
//!
//! The runtime owns what a Session MCP Snapshot is and how a live session converges on one;
//! this module resolves the snapshot from installed MCP packages and their Settings and observes
//! Host MCP health. Future session inputs such as hooks would become additional typed fields of
//! the runtime's setup rather than an open JSON map or a private Host/Agent method.

mod mcp;

#[cfg(test)]
pub(crate) use mcp::AgentSessionMcpCapabilities;
pub(crate) use mcp::{
    McpHealthStore, SessionMcpError, SessionMcpHost, SessionMcpSelection,
    observe_session_mcp_health, resolve_session_mcp, resolve_session_mcp_revision,
};
pub(crate) use ora_agent_runtime::{BarrierGuard, BarrierReason};

use crate::plugin::PluginApi;
use std::sync::Arc;

impl SessionMcpHost {
    /// Builds the host-backed catalog and configuration source used by every Session setup path.
    pub(crate) fn from_plugin_api(plugin_host: Arc<PluginApi>) -> Self {
        Self::new(plugin_host)
    }
}

pub(crate) use mcp::error::into_backend as mcp_into_backend;
