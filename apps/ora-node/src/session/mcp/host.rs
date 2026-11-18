//! Controller-provided HTTP MCP servers, attached unchanged after the plugin MCP servers of every
//! ACP setup of one execution.
//!
//! They are not plugin MCP: they never join the selection, revision, limits, refresh, health
//! probes or setup-window choice. The Node only delivers them, never connects to them, and keeps
//! their URL and headers in memory for the lifetime of the execution.
use agent_client_protocol_schema::v1::{HttpHeader, McpServer, McpServerHttp};
use ora_agent_runtime::{AgentSessionMcpCapabilities, ErrorClassification, RuntimeError};
use ora_contracts::PublicError;
use ora_node_protocol::HostMcpServer;
use std::sync::Arc;

/// Stable terminal detail when host servers need HTTP MCP the Agent did not declare. It is not a
/// plugin MCP code, so it never enters `SessionMcpErrorCode` or carries plugin identity.
pub(in crate::session) const HOST_MCP_HTTP_CAPABILITY_MISSING: &str =
    "host_mcp_http_capability_missing";

/// The servers of one execution, fixed at start. `Debug` is intentionally unavailable.
#[derive(Clone, Default)]
pub(super) struct HostServers {
    servers: Arc<Vec<McpServer>>,
    /// Raised when the gate refuses a connection. A refresh or replacement runs off the turn
    /// path and only logs its failure, so the session owner waits on this to end the execution.
    refused: Arc<tokio::sync::Notify>,
}

impl HostServers {
    /// Maps the validated start input, keeping its order.
    pub(super) fn new(servers: Vec<HostMcpServer>) -> Self {
        let servers = Arc::new(
            servers
                .into_iter()
                .map(|server| {
                    McpServer::Http(
                        McpServerHttp::new(server.name, server.url).headers(
                            server
                                .headers
                                .iter()
                                .map(|(name, value)| HttpHeader::new(name, value))
                                .collect(),
                        ),
                    )
                })
                .collect(),
        );
        Self {
            servers,
            refused: Arc::default(),
        }
    }

    /// The servers appended to every ACP setup.
    pub(super) fn servers(&self) -> Vec<McpServer> {
        self.servers.as_ref().clone()
    }

    /// Completes once the gate has refused any connection of this execution.
    pub(super) fn refused(&self) -> Arc<tokio::sync::Notify> {
        Arc::clone(&self.refused)
    }

    /// Runs after the plugin check, on every initialized connection the session sets up on, so a
    /// replacement Agent without HTTP MCP ends the execution rather than silently losing the tool.
    pub(super) fn check(
        &self,
        capabilities: AgentSessionMcpCapabilities,
    ) -> Result<(), RuntimeError> {
        if self.servers.is_empty() || capabilities.http {
            return Ok(());
        }
        // A stored permit survives until the owner starts waiting.
        self.refused.notify_one();
        Err(RuntimeError::with_source(
            ErrorClassification::Unprocessable,
            PublicError::InternalError(ora_contracts::EmptyErrorParams {}),
            "host MCP servers need HTTP MCP",
            HostMcpCapabilityMissing,
        ))
    }
}

/// Carried in the runtime error source chain so the terminal mapping needs no string parsing.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{HOST_MCP_HTTP_CAPABILITY_MISSING}")]
pub(in crate::session) struct HostMcpCapabilityMissing;

/// Whether a runtime failure is the host HTTP capability gate.
pub(in crate::session) fn is_capability_missing(error: &RuntimeError) -> bool {
    std::error::Error::source(error)
        .is_some_and(<dyn std::error::Error + 'static>::is::<HostMcpCapabilityMissing>)
}
