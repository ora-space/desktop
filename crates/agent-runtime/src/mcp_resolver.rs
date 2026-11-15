//! Pure Session MCP selection and ACP mapping shared by Desktop and Node hosts.
mod error;
mod member;
mod ports;
mod resolve;

use crate::{
    AgentSessionMcpCapabilities, SessionMcpMemberRevision, SessionMcpRevision, SessionMcpSnapshot,
    SessionMcpTransportKind,
};
pub use error::{SessionMcpError, SessionMcpErrorCode};
pub use member::{EligibleMcpMember, effective_members, find_eligible_member};
use ora_domain::SessionMcpSelection;
pub use ports::{
    InstalledMcpCandidate, McpConfigurationEligibility, SessionMcpCatalog,
    SessionMcpConfigurationSource,
};
pub use resolve::{resolve_session_mcp, resolve_session_mcp_revision, revalidate_stdio_command};
