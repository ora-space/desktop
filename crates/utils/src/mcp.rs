//! Domain-free MCP client handshake used for bounded health probes.
//!
//! This module speaks MCP over stdio (newline-delimited JSON-RPC) and Streamable HTTP. It
//! performs only `initialize`, `notifications/initialized`, and `tools/list`, then tears the
//! connection down. Callers own identity, caching, and product error codes; this crate returns
//! transport-level failure kinds without Ora domain vocabulary, and never copies OS or HTTP
//! response text into a result.

mod http;
mod protocol;
mod stdio;

#[cfg(test)]
mod tests;

pub use http::probe_http;
pub use protocol::{DEFAULT_PROBE_TIMEOUT, ProbeError, ProbeTransport};
pub use stdio::probe_stdio;

use std::time::Duration;

/// Runs one bounded MCP handshake for the given transport.
///
/// Success means `initialize` and `tools/list` both completed within `timeout`, and a stdio probe
/// has confirmed its child process was reaped before returning. Failure kinds are stable
/// classifications suitable for mapping into product error codes.
pub async fn probe(transport: ProbeTransport, timeout: Duration) -> Result<(), ProbeError> {
    match transport {
        ProbeTransport::Stdio {
            command,
            args,
            env,
            cwd,
        } => probe_stdio(command, args, env, cwd, timeout).await,
        ProbeTransport::Http { url, headers } => probe_http(url, headers, timeout).await,
    }
}
