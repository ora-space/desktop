//! Loopback Streamable HTTP MCP server exposing the host completion tool.
//!
//! The agent session receives this server as a host-owned capability. The endpoint carries a
//! server token in its path; the tool argument carries the per-attempt token issued by
//! [`super::agent_signal::AgentSignalStore`]. A call only records a pending signal; the park site
//! later binds it to the turn that attempt settled.

use super::agent_signal::AgentSignalStore;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// MCP protocol version this server answers with, matching Ora's own probe client.
pub(crate) const PROTOCOL_VERSION: &str = "2025-03-26";

/// The single tool this server exposes.
pub(crate) const COMPLETION_TOOL: &str = "ora_complete_workflow_step";

/// A running loopback MCP endpoint. Dropping it stops the accept loop.
pub(crate) struct CompletionMcpServer {
    addr: SocketAddr,
    token: String,
    shutdown: Arc<AtomicBool>,
}

impl CompletionMcpServer {
    /// Binds an ephemeral loopback port and serves until dropped.
    pub(crate) fn start(store: Arc<AgentSignalStore>) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let token = uuid::Uuid::new_v4().to_string();
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_token = token.clone();
        let thread_shutdown = shutdown.clone();
        std::thread::spawn(move || {
            while !thread_shutdown.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = serve_connection(stream, &store, &thread_token);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            addr,
            token,
            shutdown,
        })
    }

    /// Returns the endpoint URL the session MCP snapshot should carry.
    pub(crate) fn endpoint(&self) -> String {
        format!("http://{}/mcp/{}", self.addr, self.token)
    }
}

impl Drop for CompletionMcpServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

/// Largest request body accepted from the agent, protecting the loopback endpoint from a
/// malformed `Content-Length` exhausting host memory.
const MAX_REQUEST_BYTES: usize = 64 * 1024;
/// Total wall-clock budget for one connection, so a slow client cannot hold the accept loop.
const CONNECTION_DEADLINE: Duration = Duration::from_secs(10);
/// Per-read sliver of the connection budget; the remaining budget always wins.
const PER_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Serves one HTTP request on a fresh connection.
fn serve_connection(
    mut stream: TcpStream,
    store: &Arc<AgentSignalStore>,
    server_token: &str,
) -> std::io::Result<()> {
    let started = std::time::Instant::now();
    refresh_read_timeout(&stream, started)?;
    stream.set_write_timeout(Some(PER_READ_TIMEOUT))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    if !path.ends_with(&format!("/mcp/{server_token}")) {
        return write_response(&mut stream, 401, None);
    }
    let mut content_length = 0_usize;
    loop {
        refresh_read_timeout(&stream, started)?;
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("content-length:") {
            content_length = value.trim().parse().unwrap_or(0);
        } else if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            content_length = value.trim().parse().unwrap_or(0);
        }
        if content_length > MAX_REQUEST_BYTES {
            return write_response(&mut stream, 413, None);
        }
    }

    match method.as_str() {
        "DELETE" => write_response(&mut stream, 200, None),
        "POST" => {
            if content_length > MAX_REQUEST_BYTES {
                return write_response(&mut stream, 413, None);
            }
            refresh_read_timeout(&stream, started)?;
            let mut body = vec![0_u8; content_length];
            reader.read_exact(&mut body)?;
            let message: Value = match serde_json::from_slice(&body) {
                Ok(message) => message,
                Err(_) => return write_response(&mut stream, 400, None),
            };
            if message.get("id").is_none() {
                // Notifications (initialized, cancelled) have no JSON-RPC response.
                return write_response(&mut stream, 202, None);
            }
            let response = handle_rpc(store, &message);
            write_response(&mut stream, 200, Some(&response))
        }
        _ => write_response(&mut stream, 405, None),
    }
}

/// Bounds one read by whichever is smaller: the per-read sliver or the connection's budget left.
fn refresh_read_timeout(stream: &TcpStream, started: std::time::Instant) -> std::io::Result<()> {
    let remaining = CONNECTION_DEADLINE
        .checked_sub(started.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "completion request exceeded its connection deadline",
            )
        })?;
    stream.set_read_timeout(Some(remaining.min(PER_READ_TIMEOUT)))
}

/// Writes one HTTP response with an optional JSON body.
fn write_response(
    stream: &mut TcpStream,
    status: u16,
    body: Option<&Value>,
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        _ => "Error",
    };
    let payload = body.map(Value::to_string).unwrap_or_default();
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

/// Handles one JSON-RPC request; notifications are filtered by the caller.
fn handle_rpc(store: &Arc<AgentSignalStore>, message: &Value) -> Value {
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    match method {
        "initialize" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "ora-workflow-completion", "version": "0.1.0" }
            }
        }),
        "ping" => json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
        "tools/list" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": [{
                    "name": COMPLETION_TOOL,
                    "description": "Report that this workflow step is complete. Pass the attempt token from the step boundary.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "attemptToken": { "type": "string" }
                        },
                        "required": ["attemptToken"],
                        "additionalProperties": false
                    }
                }]
            }
        }),
        "tools/call" => {
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            if name != COMPLETION_TOOL {
                return rpc_error(id, -32602, "unknown tool");
            }
            let Some(token) = params
                .get("arguments")
                .and_then(|arguments| arguments.get("attemptToken"))
                .and_then(Value::as_str)
            else {
                return rpc_error(id, -32602, "attemptToken is required");
            };
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis() as i64)
                .unwrap_or_default();
            match store.accept(token, now) {
                Ok(signal_id) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{ "type": "text", "text": format!("accepted:{signal_id}") }],
                        "isError": false
                    }
                }),
                Err(_) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{ "type": "text", "text": "unknown attempt token" }],
                        "isError": true
                    }
                }),
            }
        }
        _ => rpc_error(id, -32601, "method not found"),
    }
}

/// Builds one JSON-RPC error response.
fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

#[cfg(test)]
mod tests {
    use super::{COMPLETION_TOOL, CompletionMcpServer, MAX_REQUEST_BYTES};
    use crate::workflow::run::completion_snapshot::TurnKey;
    use crate::workflow::run::interactive::agent_signal::AgentSignalStore;
    use serde_json::{Value, json};
    use std::io::{self, Read, Write};
    use std::net::TcpStream;
    use std::sync::Arc;
    use std::time::Duration;

    /// Sends one JSON-RPC POST to the endpoint and returns the status and decoded body.
    ///
    /// The server bounds how long it waits for a request; a heavily loaded CI runner can delay the
    /// test client past that window and make the loopback round trip fail transiently, so retry.
    fn post(endpoint: &str, body: &Value) -> (u16, Value) {
        let mut last = None;
        for _ in 0..3 {
            match post_once(endpoint, body) {
                Ok(result) => return result,
                Err(error) => {
                    last = Some(error);
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
        panic!(
            "loopback MCP request failed after retries: {}",
            last.expect("at least one attempt")
        );
    }

    /// Performs one loopback request, surfacing transport and response-shape failures.
    fn post_once(endpoint: &str, body: &Value) -> io::Result<(u16, Value)> {
        let rest = endpoint
            .strip_prefix("http://")
            .ok_or_else(|| io::Error::other("endpoint is not http"))?;
        let (addr, path) = rest
            .split_once('/')
            .ok_or_else(|| io::Error::other("endpoint has no path"))?;
        let mut stream = TcpStream::connect(addr)?;
        let payload = body.to_string();
        write!(
            stream,
            "POST /{path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
            payload.len()
        )?;
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        let (head, body) = response
            .split_once("\r\n\r\n")
            .ok_or_else(|| io::Error::other("response has no body"))?;
        let status = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|status| status.parse().ok())
            .ok_or_else(|| io::Error::other("response has no status"))?;
        // Error responses carry no JSON body; the caller asserts the status in that case.
        let body = serde_json::from_str(body).unwrap_or(Value::Null);
        Ok((status, body))
    }

    #[test]
    fn initialize_and_tools_list_expose_the_completion_tool() {
        let store = Arc::new(AgentSignalStore::new());
        let server = CompletionMcpServer::start(store).expect("server");
        let endpoint = server.endpoint();

        let (status, initialize) = post(
            &endpoint,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": { "protocolVersion": "2025-03-26", "capabilities": {} }
            }),
        );
        assert_eq!(status, 200);
        assert_eq!(initialize["result"]["protocolVersion"], json!("2025-03-26"));

        let (status, tools) = post(
            &endpoint,
            &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }),
        );
        assert_eq!(status, 200);
        assert_eq!(tools["result"]["tools"][0]["name"], json!(COMPLETION_TOOL));
    }

    #[test]
    fn a_tool_call_accepts_the_attempt_token_and_binds_to_its_turn() {
        let store = Arc::new(AgentSignalStore::new());
        let token = store.issue("node-1", "attempt-1");
        let server = CompletionMcpServer::start(store.clone()).expect("server");

        let (status, response) = post(
            &server.endpoint(),
            &json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": COMPLETION_TOOL,
                    "arguments": { "attemptToken": token }
                }
            }),
        );
        assert_eq!(status, 200);
        assert_eq!(response["result"]["isError"], json!(false));

        let signal = store
            .bind(
                "node-1",
                "attempt-1",
                TurnKey {
                    user_seq: 1,
                    ended_seq: 2,
                },
            )
            .expect("bound signal");
        assert!(!signal.signal_id.is_empty());
    }

    #[test]
    fn an_unknown_token_is_a_tool_error_and_a_wrong_path_is_unauthorized() {
        let store = Arc::new(AgentSignalStore::new());
        let server = CompletionMcpServer::start(store).expect("server");

        let (status, response) = post(
            &server.endpoint(),
            &json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {
                    "name": COMPLETION_TOOL,
                    "arguments": { "attemptToken": "missing" }
                }
            }),
        );
        assert_eq!(status, 200);
        assert_eq!(response["result"]["isError"], json!(true));

        let tampered = server.endpoint().replace("mcp/", "mcp/wrong-");
        let (status, _) = post(
            &tampered,
            &json!({ "jsonrpc": "2.0", "id": 5, "method": "ping" }),
        );
        assert_eq!(status, 401);
    }

    #[test]
    fn an_oversized_request_is_rejected_before_reading_it() {
        let store = Arc::new(AgentSignalStore::new());
        let server = CompletionMcpServer::start(store).expect("server");
        let rest = server
            .endpoint()
            .strip_prefix("http://")
            .expect("http endpoint")
            .to_string();
        let (addr, path) = rest.split_once('/').expect("path");
        let mut stream = TcpStream::connect(addr).expect("connect");
        write!(
            stream,
            "POST /{path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            MAX_REQUEST_BYTES + 1
        )
        .expect("write headers");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read response");
        assert!(
            response.starts_with("HTTP/1.1 413"),
            "unexpected response: {response:?}"
        );
    }
}
