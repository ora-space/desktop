//! Component tests for the domain-free MCP probe client.
//!
//! stdio fixtures respawn this very test binary as the fake server so the success/failure matrix
//! needs no external interpreter; HTTP fixtures use a raw loopback TCP listener.

use super::{ProbeError, ProbeTransport, probe};
use pretty_assertions::assert_eq;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use url::Url;

/// Selects the fake-server behavior when this test binary is respawned as a child process.
const FAKE_SERVER_ENV: &str = "ORA_UTILS_MCP_FAKE_SERVER";

/// Spawns this test binary as a fake MCP stdio server in the given mode.
///
/// The harness noise the child prints (`running 1 test`, …) is not JSON, and the probe reader
/// skips non-JSON lines, so protocol frames are the only payloads ever accepted.
fn fake_server_transport(mode: &str) -> ProbeTransport {
    ProbeTransport::Stdio {
        command: std::env::current_exe().expect("test binary path"),
        args: vec![
            "--exact".to_string(),
            "mcp::tests::fake_mcp_stdio_server".to_string(),
            "--nocapture".to_string(),
        ],
        env: vec![(FAKE_SERVER_ENV.to_string(), mode.to_string())],
        cwd: None,
    }
}

/// Fake MCP stdio server entry point; without the env flag it is a no-op passing test.
#[test]
fn fake_mcp_stdio_server() {
    let Ok(mode) = std::env::var(FAKE_SERVER_ENV) else {
        return;
    };
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let read_message = || -> Option<serde_json::Value> {
        let mut line = String::new();
        let read = stdin.lock().read_line(&mut line).expect("read stdin");
        if read == 0 {
            return None;
        }
        Some(serde_json::from_str(line.trim()).expect("request json"))
    };
    let mut write_message = |message: serde_json::Value| {
        writeln!(stdout, "{message}").expect("write stdout");
        stdout.flush().expect("flush stdout");
    };
    match mode.as_str() {
        "exit" => std::process::exit(0),
        "hang" => {
            // Read forever without answering; the probe's hard timeout must kill this process.
            while read_message().is_some() {}
        }
        "wrong-id" => {
            let request = read_message().expect("initialize");
            assert_eq!(request["method"], "initialize");
            write_message(serde_json::json!({
                "jsonrpc": "2.0",
                "id": 99,
                "result": { "protocolVersion": "2025-03-26", "capabilities": {} }
            }));
        }
        "ok" | "tools-error" | "slow-exit" => {
            let request = read_message().expect("initialize");
            assert_eq!(request["method"], "initialize");
            write_message(serde_json::json!({
                "jsonrpc": "2.0",
                "id": request["id"],
                "result": {
                    "protocolVersion": "2025-03-26",
                    "capabilities": {},
                    "serverInfo": { "name": "fake", "version": "0" }
                }
            }));
            let notification = read_message().expect("initialized");
            assert_eq!(notification["method"], "notifications/initialized");
            let request = read_message().expect("tools/list");
            assert_eq!(request["method"], "tools/list");
            if mode == "tools-error" {
                write_message(serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": request["id"],
                    "error": { "code": -32000, "message": "tools unavailable" }
                }));
            } else {
                write_message(serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": request["id"],
                    "result": { "tools": [] }
                }));
            }
            // Stay alive until the probe closes stdin so clean teardown is exercised; the slow
            // variant then lingers, which only a probe that waits for the child can outlast.
            while read_message().is_some() {}
            if mode == "slow-exit" {
                std::thread::sleep(Duration::from_millis(300));
            }
        }
        other => panic!("unknown fake server mode {other}"),
    }
}

#[tokio::test]
async fn stdio_probe_succeeds_for_well_behaved_server() {
    let result = probe(fake_server_transport("ok"), Duration::from_secs(15)).await;
    assert_eq!(result, Ok(()));
}

/// A probe returns only after the child has exited, which is what makes reclamation certain.
#[tokio::test]
async fn stdio_probe_waits_for_child_exit_after_a_handshake() {
    let started = Instant::now();
    let result = probe(fake_server_transport("slow-exit"), Duration::from_secs(15)).await;
    assert_eq!(result, Ok(()));
    assert!(
        started.elapsed() >= Duration::from_millis(250),
        "probe returned before the child exited: {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn stdio_probe_maps_missing_binary_to_spawn_failed() {
    let result = probe(
        ProbeTransport::Stdio {
            command: PathBuf::from("definitely-not-an-mcp-binary-xyz"),
            args: Vec::new(),
            env: Vec::new(),
            cwd: None,
        },
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(result, Err(ProbeError::SpawnFailed));
}

#[tokio::test]
async fn stdio_probe_maps_early_exit_to_exited_prematurely() {
    let result = probe(fake_server_transport("exit"), Duration::from_secs(15)).await;
    assert_eq!(result, Err(ProbeError::ExitedPrematurely));
}

#[tokio::test]
async fn stdio_probe_maps_wrong_id_answer_to_handshake_failed() {
    let result = probe(fake_server_transport("wrong-id"), Duration::from_secs(15)).await;
    assert_eq!(result, Err(ProbeError::HandshakeFailed));
}

#[tokio::test]
async fn stdio_probe_maps_tools_list_error_to_tools_unavailable() {
    let result = probe(
        fake_server_transport("tools-error"),
        Duration::from_secs(15),
    )
    .await;
    assert_eq!(result, Err(ProbeError::ToolsUnavailable));
}

/// A hung server must hit the hard timeout, and the kill + reap must not outlive it: the probe
/// returning promptly after the deadline is the observable proof no orphan is left running.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stdio_probe_maps_hang_to_timeout_and_reaps_promptly() {
    let started = Instant::now();
    let result = probe(fake_server_transport("hang"), Duration::from_millis(300)).await;
    assert_eq!(result, Err(ProbeError::Timeout));
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "timeout probe must kill and reap instead of lingering: {:?}",
        started.elapsed()
    );
}

/// One raw HTTP request read from a fresh connection: request line, headers, then the body.
struct RawRequest {
    body: String,
}

/// One canned response the fixture serves for the next connection.
struct RawResponse {
    status: &'static str,
    content_type: &'static str,
    body: String,
    session_id: Option<&'static str>,
}

impl RawResponse {
    fn json(body: serde_json::Value) -> Self {
        Self {
            status: "200 OK",
            content_type: "application/json",
            body: body.to_string(),
            session_id: Some("session-1"),
        }
    }

    fn sse(payload: serde_json::Value) -> Self {
        Self {
            status: "200 OK",
            content_type: "text/event-stream",
            body: format!("event: message\ndata: {payload}\n\n"),
            session_id: Some("session-1"),
        }
    }

    fn empty(status: &'static str) -> Self {
        Self {
            status,
            content_type: "text/plain",
            body: String::new(),
            session_id: None,
        }
    }
}

/// Runs a loopback HTTP server that answers one request per queued response, calling `inspect`
/// with each received body. Returns the endpoint URL plus the received bodies on join.
fn serve_responses(
    responses: Vec<RawResponse>,
) -> (Url, mpsc::Receiver<String>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (body_tx, body_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        for response in responses {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            let request = match read_request(stream.try_clone().expect("clone stream")) {
                Some(request) => request,
                None => return,
            };
            let _ = body_tx.send(request.body);
            let mut stream = stream;
            let mut head = format!(
                "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
                response.status,
                response.content_type,
                response.body.len()
            );
            if let Some(session_id) = response.session_id {
                head.push_str(&format!("Mcp-Session-Id: {session_id}\r\n"));
            }
            head.push_str("\r\n");
            if stream
                .write_all(head.as_bytes())
                .and_then(|_| stream.write_all(response.body.as_bytes()))
                .is_err()
            {
                return;
            }
        }
    });
    (
        Url::parse(&format!("http://{addr}/mcp")).expect("url"),
        body_rx,
        handle,
    )
}

/// Reads one HTTP request with a Content-Length body; returns None on protocol surprise.
fn read_request(stream: std::net::TcpStream) -> Option<RawRequest> {
    let mut reader = BufReader::new(stream);
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = trimmed.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = value.trim().parse().ok()?;
        }
    }
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body).ok()?;
    Some(RawRequest {
        body: String::from_utf8(body).ok()?,
    })
}

fn initialize_ok() -> RawResponse {
    RawResponse::json(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "serverInfo": { "name": "http-fake", "version": "0" }
        }
    }))
}

fn tools_ok() -> RawResponse {
    RawResponse::json(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "result": { "tools": [] }
    }))
}

/// A complete JSON handshake, the notification ack, and the session teardown DELETE.
#[tokio::test]
async fn http_probe_succeeds_for_json_responses() {
    let (url, bodies, server) = serve_responses(vec![
        initialize_ok(),
        RawResponse::empty("202 Accepted"),
        tools_ok(),
        RawResponse::empty("200 OK"),
    ]);
    let result = probe(
        ProbeTransport::Http {
            url,
            headers: Vec::new(),
        },
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(result, Ok(()));
    server.join().expect("server thread");
    let bodies: Vec<String> = [bodies.recv(), bodies.recv(), bodies.recv()]
        .into_iter()
        .map(|body| body.expect("request body"))
        .collect();
    assert!(bodies[0].contains("\"initialize\""), "{}", bodies[0]);
    assert!(
        bodies[1].contains("notifications/initialized"),
        "{}",
        bodies[1]
    );
    assert!(bodies[2].contains("\"tools/list\""), "{}", bodies[2]);
}

/// Streamable HTTP servers may answer with an SSE frame instead of a bare JSON body.
#[tokio::test]
async fn http_probe_accepts_sse_framed_responses() {
    let (url, _bodies, server) = serve_responses(vec![
        RawResponse::sse(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "protocolVersion": "2025-03-26", "capabilities": {} }
        })),
        RawResponse::empty("202 Accepted"),
        RawResponse::sse(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": { "tools": [] }
        })),
        RawResponse::empty("200 OK"),
    ]);
    let result = probe(
        ProbeTransport::Http {
            url,
            headers: Vec::new(),
        },
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(result, Ok(()));
    server.join().expect("server thread");
}

#[tokio::test]
async fn http_probe_maps_unauthorized() {
    let (url, _bodies, server) = serve_responses(vec![RawResponse::empty("401 Unauthorized")]);
    let result = probe(
        ProbeTransport::Http {
            url,
            // Deliberately secret-looking: the failure classification must never carry it back.
            headers: vec![("Authorization".into(), "Bearer secret-key".into())],
        },
        Duration::from_secs(3),
    )
    .await;
    assert_eq!(result, Err(ProbeError::HttpUnauthorized));
    server.join().expect("server thread");
}

#[tokio::test]
async fn http_probe_maps_forbidden_to_unauthorized() {
    let (url, _bodies, server) = serve_responses(vec![RawResponse::empty("403 Forbidden")]);
    let result = probe(
        ProbeTransport::Http {
            url,
            headers: Vec::new(),
        },
        Duration::from_secs(3),
    )
    .await;
    assert_eq!(result, Err(ProbeError::HttpUnauthorized));
    server.join().expect("server thread");
}

#[tokio::test]
async fn http_probe_maps_server_error() {
    let (url, _bodies, server) =
        serve_responses(vec![RawResponse::empty("503 Service Unavailable")]);
    let result = probe(
        ProbeTransport::Http {
            url,
            headers: Vec::new(),
        },
        Duration::from_secs(3),
    )
    .await;
    assert_eq!(result, Err(ProbeError::HttpServerError));
    server.join().expect("server thread");
}

/// A 200 with a non-MCP body is a handshake failure: the endpoint answered but is not MCP.
#[tokio::test]
async fn http_probe_maps_non_mcp_body_to_handshake_failed() {
    let (url, _bodies, server) = serve_responses(vec![RawResponse {
        status: "200 OK",
        content_type: "text/html",
        body: "<html>not an mcp server</html>".to_string(),
        session_id: None,
    }]);
    let result = probe(
        ProbeTransport::Http {
            url,
            headers: Vec::new(),
        },
        Duration::from_secs(3),
    )
    .await;
    assert_eq!(result, Err(ProbeError::HandshakeFailed));
    server.join().expect("server thread");
}

/// A JSON-RPC error on tools/list after a good initialize means the tools surface failed.
#[tokio::test]
async fn http_probe_maps_tools_list_error_to_tools_unavailable() {
    let (url, _bodies, server) = serve_responses(vec![
        initialize_ok(),
        RawResponse::empty("202 Accepted"),
        RawResponse::json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "error": { "code": -32000, "message": "tools unavailable" }
        })),
    ]);
    let result = probe(
        ProbeTransport::Http {
            url,
            headers: Vec::new(),
        },
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(result, Err(ProbeError::ToolsUnavailable));
    server.join().expect("server thread");
}

#[tokio::test]
async fn http_probe_maps_unreachable() {
    // A peer that accepts and immediately closes cannot complete any HTTP exchange. This is a
    // deterministic stand-in for a refused or reset connection, which behaves differently across
    // host firewalls (some silently drop instead of refusing).
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = thread::spawn(move || {
        // Exactly one connection is attempted before the client gives up.
        if let Ok((stream, _)) = listener.accept() {
            drop(stream);
        }
    });
    let result = probe(
        ProbeTransport::Http {
            url: Url::parse(&format!("http://{addr}/mcp")).expect("url"),
            headers: Vec::new(),
        },
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(result, Err(ProbeError::HttpUnreachable));
    server.join().expect("server thread");
}

/// A peer that accepts but never answers must hit the hard timeout, not a per-request one.
#[tokio::test]
async fn http_probe_maps_silent_peer_to_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            // Read the request but keep the connection open without ever answering, so only the
            // probe's own deadline can end the exchange.
            let handled = stream.try_clone().expect("clone stream");
            let _ = read_request(handled);
            thread::sleep(Duration::from_secs(2));
            drop(stream);
        }
    });
    let started = Instant::now();
    let result = probe(
        ProbeTransport::Http {
            url: Url::parse(&format!("http://{addr}/mcp")).expect("url"),
            headers: Vec::new(),
        },
        Duration::from_millis(300),
    )
    .await;
    assert_eq!(result, Err(ProbeError::Timeout));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "hard timeout must bound the whole handshake: {:?}",
        started.elapsed()
    );
    server.join().expect("server thread");
}
