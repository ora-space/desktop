//! Session MCP types, resolver, and in-memory live-session convergence.

mod error;
mod host;
mod live;
mod resolve;

#[cfg(test)]
mod tests;

pub(crate) use error::SessionMcpError;
pub(crate) use host::{
    InstalledMcpCandidate, McpConfigurationEligibility, SessionMcpCatalog,
    SessionMcpConfigurationSource, SessionMcpHost,
};
pub(crate) use live::{LiveMcpEvent, LiveMcpPromptAdmission, LiveMcpState};
pub(crate) use resolve::{resolve_session_mcp, resolve_session_mcp_revision};

use agent_client_protocol_schema::v1::McpServer;
use ora_domain::PluginId;
use semver::Version;
use std::fmt;

/// Agent capabilities that Session MCP setup must consult before sending ACP frames.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AgentSessionMcpCapabilities {
    pub load_session: bool,
    pub http: bool,
}

impl AgentSessionMcpCapabilities {
    /// Builds the capability pair read from one initialized ACP connection.
    pub(crate) const fn new(load_session: bool, http: bool) -> Self {
        Self { load_session, http }
    }
}

/// Secret-free identity of one Effective MCP Set member.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct SessionMcpMemberRevision {
    pub plugin_id: PluginId,
    pub package_version: Version,
    pub configuration_revision: u64,
    pub transport: SessionMcpTransportKind,
}

/// Distinguishes transport kinds in the Desired revision without carrying Setting values.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum SessionMcpTransportKind {
    Stdio,
    Http,
}

impl SessionMcpTransportKind {
    /// Stable wire token used in capability and setup errors.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Http => "http",
        }
    }
}

/// Secret-free content identity of one complete Session MCP Snapshot.
///
/// Equality is the live-session digest: it is compared in memory and must never include Setting
/// values, ACP env, or HTTP headers.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SessionMcpRevision {
    members: Vec<SessionMcpMemberRevision>,
}

impl SessionMcpRevision {
    /// Builds a revision from members already ordered by canonical Plugin ID.
    pub(crate) fn new(members: Vec<SessionMcpMemberRevision>) -> Self {
        Self { members }
    }

    /// Members in canonical Plugin ID order.
    pub(crate) fn members(&self) -> &[SessionMcpMemberRevision] {
        &self.members
    }

    /// Whether the corresponding Snapshot would contain any MCP servers.
    pub(crate) fn is_empty(&self) -> bool {
        self.members().is_empty()
    }
}

/// One ACP `mcpServers` payload together with the secret-free revision that named it.
///
/// Debug omits server env and headers so a log of the snapshot cannot leak Setting values.
#[derive(Clone)]
pub(crate) struct SessionMcpSnapshot {
    servers: Vec<McpServer>,
    revision: SessionMcpRevision,
}

impl SessionMcpSnapshot {
    /// Builds a snapshot whose servers and revision were produced together.
    pub(crate) fn new(servers: Vec<McpServer>, revision: SessionMcpRevision) -> Self {
        Self { servers, revision }
    }

    /// ACP servers in canonical Plugin ID order.
    pub(crate) fn servers(&self) -> &[McpServer] {
        &self.servers
    }

    /// Consumes the snapshot into the ACP list that may be sent exactly once.
    pub(crate) fn into_servers(self) -> Vec<McpServer> {
        self.servers
    }

    /// Secret-free identity of this snapshot.
    pub(crate) fn revision(&self) -> &SessionMcpRevision {
        &self.revision
    }
}

impl fmt::Debug for SessionMcpSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionMcpSnapshot")
            .field("revision", &self.revision)
            .field("server_count", &self.servers.len())
            .finish()
    }
}
