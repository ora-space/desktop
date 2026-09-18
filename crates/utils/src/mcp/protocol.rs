//! Shared MCP JSON-RPC shapes and probe failure classifications.

use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;
use url::Url;

/// Default hard timeout for one probe. Must stay well below a typical 30s session-setup budget.
pub const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_secs(8);

/// Protocol version advertised by this client during `initialize`.
pub(super) const PROTOCOL_VERSION: &str = "2025-03-26";

/// Inputs for one probe. Values may include secrets; they must never appear in `ProbeError`.
#[derive(Clone, Debug)]
pub enum ProbeTransport {
    /// Spawn a local process and speak newline-delimited JSON-RPC on its stdio pipes.
    Stdio {
        command: PathBuf,
        args: Vec<String>,
        env: Vec<(String, String)>,
        cwd: Option<PathBuf>,
    },
    /// POST JSON-RPC to a Streamable HTTP endpoint.
    Http {
        url: Url,
        headers: Vec<(String, String)>,
    },
}

/// Stable, secret-free classification of a failed probe.
///
/// Variants map 1:1 onto the product health error-code family without carrying OS text, HTTP
/// bodies, argv, env, or headers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeError {
    /// The stdio process could not be started.
    SpawnFailed,
    /// The process exited before the handshake finished.
    ExitedPrematurely,
    /// `initialize` produced no usable result or a protocol error.
    HandshakeFailed,
    /// The hard timeout elapsed before both methods completed.
    Timeout,
    /// Handshake succeeded but `tools/list` failed.
    ToolsUnavailable,
    /// DNS, TCP, TLS, or other reachability failure for HTTP.
    HttpUnreachable,
    /// HTTP 401 or 403.
    HttpUnauthorized,
    /// Other HTTP 4xx or 5xx.
    HttpServerError,
}

/// Builds the JSON-RPC `initialize` request body.
pub(super) fn initialize_request(id: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": {
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {
                "name": "mcp-probe",
                "version": "0.0.1"
            }
        }
    })
}

/// Builds the JSON-RPC `notifications/initialized` notification (no id).
pub(super) fn initialized_notification() -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    })
}

/// Builds the JSON-RPC `tools/list` request body.
pub(super) fn tools_list_request(id: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/list",
        "params": {}
    })
}

/// Minimal JSON-RPC response used to accept or reject a handshake step.
#[derive(Debug, Deserialize)]
pub(super) struct JsonRpcResponse {
    pub id: Option<Value>,
    pub result: Option<Value>,
    pub error: Option<Value>,
}

/// Confirms a response is a successful result for the expected request id.
///
/// Sends both the `result` and `error` fields to the check: a JSON-RPC error is never accepted
/// even when an id matches.
pub(super) fn accept_result(response: &JsonRpcResponse, expected_id: u64) -> bool {
    if response.error.is_some() || response.result.is_none() {
        return false;
    }
    match &response.id {
        Some(Value::Number(number)) => number.as_u64() == Some(expected_id),
        Some(Value::String(text)) => text.parse::<u64>().ok() == Some(expected_id),
        _ => false,
    }
}

/// Serializes one JSON-RPC message followed by a newline for stdio framing.
pub(super) fn encode_line(message: &Value) -> Result<Vec<u8>, ProbeError> {
    let mut bytes = serde_json::to_vec(message).map_err(|_| ProbeError::HandshakeFailed)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Parses one JSON object from a stdio response line.
pub(super) fn decode_line(line: &str) -> Result<JsonRpcResponse, ProbeError> {
    serde_json::from_str(line.trim()).map_err(|_| ProbeError::HandshakeFailed)
}

#[cfg(test)]
mod tests {
    use super::{
        accept_result, decode_line, encode_line, initialize_request, initialized_notification,
        tools_list_request,
    };
    use pretty_assertions::assert_eq;

    /// Only a successful result with the exact expected id is accepted.
    #[test]
    fn accepts_only_matching_successful_results() {
        let accepts = |json: &str, expected: u64| {
            accept_result(&decode_line(json).expect("decode response"), expected)
        };
        assert!(accepts(
            r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#,
            1
        ));
        assert!(accepts(r#"{"jsonrpc":"2.0","id":"1","result":{}}"#, 1));
        // A mismatched id, a JSON-RPC error, and a missing result are all rejected.
        assert!(!accepts(r#"{"jsonrpc":"2.0","id":2,"result":{}}"#, 1));
        assert!(!accepts(
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000}}"#,
            1
        ));
        assert!(!accepts(r#"{"jsonrpc":"2.0","id":1}"#, 1));
        // Notifications and server requests carry no id and are never accepted.
        assert!(!accepts(r#"{"jsonrpc":"2.0","method":"note"}"#, 1));
    }

    /// Framing round-trips through the newline-delimited encoding.
    #[test]
    fn frames_messages_with_a_trailing_newline() {
        let encoded = encode_line(&initialize_request(1)).expect("encode");
        assert_eq!(*encoded.last().expect("newline"), b'\n');
        let text = String::from_utf8(encoded).expect("utf8");
        assert!(text.contains("\"initialize\""));
        let decoded = decode_line(&text).expect("decode");
        assert_eq!(decoded.id.is_some(), true);
    }

    /// Requests carry the methods the probe contract restricts itself to.
    #[test]
    fn builds_only_the_initialize_notification_and_tools_list() {
        assert_eq!(
            initialize_request(1)["method"],
            serde_json::json!("initialize")
        );
        assert_eq!(
            initialized_notification()["method"],
            serde_json::json!("notifications/initialized")
        );
        assert_eq!(
            tools_list_request(2)["method"],
            serde_json::json!("tools/list")
        );
        assert!(initialized_notification().get("id").is_none());
    }

    /// The hard timeout must stay far below a session-setup budget, so a probe can never pose as
    /// setup and block the path that owns the 30s window.
    #[test]
    fn default_probe_timeout_stays_well_below_a_session_setup_budget() {
        let budget = std::time::Duration::from_secs(30);
        assert!(super::DEFAULT_PROBE_TIMEOUT < budget / 3);
    }
}
