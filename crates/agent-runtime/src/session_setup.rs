//! Session setup values the runtime sends at every ACP `session/new` and `session/load`.
//!
//! The runtime owns what a Session MCP Snapshot *is* and how a live session converges on one; the
//! host owns where the snapshot comes from (see [`crate::SessionSetup`]). The pure resolver is
//! shared; Desktop catalog discovery, Setting storage and health probes remain host responsibilities.

mod barrier;
mod live;

pub use barrier::{AgentSessionBarrier, AgentSessionBarriers, BarrierGuard, BarrierReason};
pub use live::{LiveMcpEvent, LiveMcpPromptAdmission, LiveMcpState};

use agent_client_protocol_schema::v1::McpServer;
use ora_domain::PluginId;
use semver::Version;
use std::fmt;

/// Agent capabilities that Session MCP setup must consult before sending ACP frames.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentSessionMcpCapabilities {
    pub load_session: bool,
    pub http: bool,
}

impl AgentSessionMcpCapabilities {
    /// Builds the capability pair read from one initialized ACP connection.
    pub const fn new(load_session: bool, http: bool) -> Self {
        Self { load_session, http }
    }
}

/// Secret-free identity of one Effective MCP Set member.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct SessionMcpMemberRevision {
    pub plugin_id: PluginId,
    pub package_version: Version,
    pub configuration_revision: u64,
    pub transport: SessionMcpTransportKind,
}

/// Distinguishes transport kinds in the Desired revision without carrying Setting values.
///
/// `Hash` lets health identities key a process-local cache by transport without a second enum.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum SessionMcpTransportKind {
    Stdio,
    Http,
}

impl SessionMcpTransportKind {
    /// Stable wire token used in capability and setup errors.
    pub const fn as_str(self) -> &'static str {
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
pub struct SessionMcpRevision {
    members: Vec<SessionMcpMemberRevision>,
}

impl SessionMcpRevision {
    /// Builds a revision from members already ordered by canonical Plugin ID.
    pub fn new(members: Vec<SessionMcpMemberRevision>) -> Self {
        Self { members }
    }

    /// Members in canonical Plugin ID order.
    pub fn members(&self) -> &[SessionMcpMemberRevision] {
        &self.members
    }

    /// Whether the corresponding Snapshot would contain any MCP servers.
    pub fn is_empty(&self) -> bool {
        self.members().is_empty()
    }
}

/// One ACP `mcpServers` payload together with the secret-free revision that named it.
///
/// Debug omits server env and headers so a log of the snapshot cannot leak Setting values.
#[derive(Clone)]
pub struct SessionMcpSnapshot {
    servers: Vec<McpServer>,
    revision: SessionMcpRevision,
}

impl SessionMcpSnapshot {
    /// Builds a snapshot whose servers and revision were produced together.
    pub fn new(servers: Vec<McpServer>, revision: SessionMcpRevision) -> Self {
        Self { servers, revision }
    }

    /// ACP servers in canonical Plugin ID order.
    pub fn servers(&self) -> &[McpServer] {
        &self.servers
    }

    /// Consumes the snapshot into the ACP list that may be sent exactly once.
    pub fn into_servers(self) -> Vec<McpServer> {
        self.servers
    }

    /// Secret-free identity of this snapshot.
    pub fn revision(&self) -> &SessionMcpRevision {
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
