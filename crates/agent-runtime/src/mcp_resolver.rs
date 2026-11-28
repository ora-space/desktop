//! Pure Session MCP selection and ACP mapping shared by Desktop and Node hosts.
mod error;
mod member;
mod ports;
mod resolve;

use crate::{
    AgentSessionMcpCapabilities, SessionMcpMemberRevision, SessionMcpRevision, SessionMcpSnapshot,
    SessionMcpTransportKind,
};
use agent_client_protocol_schema::v1::McpServer;
pub use error::{SessionMcpError, SessionMcpErrorCode};
pub use member::{EligibleMcpMember, effective_members, find_eligible_member};
use ora_domain::SessionMcpSelection;
pub use ports::{
    InstalledMcpCandidate, McpConfigurationEligibility, SessionMcpCatalog,
    SessionMcpConfigurationSource,
};
pub use resolve::{resolve_session_mcp, resolve_session_mcp_revision, revalidate_stdio_command};

/// Collects the runtime values a set of ACP `McpServer`s delivers to the Agent, for redaction:
/// stdio args and env values, and http/sse url and header values (ADR D4).
pub(super) fn redactable_mcp_values(servers: &[McpServer]) -> Vec<String> {
    let mut values = Vec::new();
    for server in servers {
        match server {
            McpServer::Stdio(stdio) => {
                values.extend(stdio.args.iter().cloned());
                values.extend(stdio.env.iter().map(|env| env.value.clone()));
            }
            McpServer::Http(http) => {
                values.push(http.url.clone());
                values.extend(http.headers.iter().map(|header| header.value.clone()));
            }
            McpServer::Sse(sse) => {
                values.push(sse.url.clone());
                values.extend(sse.headers.iter().map(|header| header.value.clone()));
            }
            // New transports carry no redactable runtime values of their own.
            _ => {}
        }
    }
    values
}
