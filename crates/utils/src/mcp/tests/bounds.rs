//! Direct transport evidence for the MCP probe's credential and allocation boundaries.
//! See specs/test-cases/node/agent-runtime/session-mcp-lifecycle.md#http-probes-follow-no-redirect-use-no-proxy-and-read-at-most-one-mib.

use super::super::protocol::MAX_RESPONSE_BYTES;
use super::super::{ProbeError, ProbeTransport, probe};
use pretty_assertions::assert_eq;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use url::Url;

struct Fixture {
    url: Url,
    stop: oneshot::Sender<()>,
    task: JoinHandle<Vec<String>>,
}

impl Fixture {
    /// Serves finite scripted responses and records even unexpected redirect requests.
    async fn start(responses: Vec<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        let (stop, mut stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            loop {
                let stream = tokio::select! {
                    _ = &mut stopped => break,
                    connection = listener.accept() => connection.expect("accept").0,
                };
                let mut reader = BufReader::new(stream);
                let mut request = String::new();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).await.expect("read header") == 0 {
                        return requests;
                    }
                    request.push_str(&line);
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().expect("length");
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).await.expect("read body");
                request.push_str(std::str::from_utf8(&body).expect("utf8"));
                requests.push(request);
                let response = responses
                    .get(requests.len() - 1)
                    .cloned()
                    .unwrap_or_else(|| response("500 Unexpected", "", "", ""));
                let _ = reader.get_mut().write_all(response.as_bytes()).await;
                let _ = reader.get_mut().shutdown().await;
            }
            requests
        });
        Self {
            url: Url::parse(&format!("http://{address}/mcp")).expect("url"),
            stop,
            task,
        }
    }

    /// Stops an idle listener without timing-dependent accept loops or leaked fixture tasks.
    async fn finish(self) -> Vec<String> {
        let _ = self.stop.send(());
        self.task.await.expect("server task")
    }

    /// Uses the public transport entry point with a header that must never follow a redirect.
    async fn probe(&self) -> Result<(), ProbeError> {
        probe(
            ProbeTransport::Http {
                url: self.url.clone(),
                headers: vec![("X-API-Key".into(), "fixture-secret".into())],
            },
            Duration::from_secs(5),
        )
        .await
    }
}

/// Makes connection-close framing explicit so tests do not depend on connection pooling.
fn response(status: &str, content_type: &str, body: &str, headers: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
        body.len()
    )
}

/// The only three MCP messages and an optional DELETE comprise a successful probe.
fn success() -> Vec<String> {
    vec![
        response(
            "200 OK",
            "application/json",
            r#"{"id":1,"result":{}}"#,
            "Mcp-Session-Id: test-session\r\n",
        ),
        response("202 Accepted", "", "", ""),
        response(
            "200 OK",
            "application/json",
            r#"{"id":2,"result":{"tools":[]}}"#,
            "",
        ),
        response("200 OK", "", "", ""),
    ]
}

/// Redirect rejection applies to initialize, notification, tools/list and DELETE, including
/// same-origin targets that would retain arbitrary secret headers under reqwest defaults.
#[tokio::test]
async fn rejects_redirects_at_every_stage_without_contacting_the_target() {
    for stage in 0..4 {
        for external in [false, true] {
            let target = Fixture::start(success()).await;
            let location = if external {
                target.url.as_str()
            } else {
                "/redirected"
            };
            let mut replies = success();
            replies[stage] = response(
                "307 Temporary Redirect",
                "",
                "",
                &format!("Location: {location}\r\n"),
            );
            replies.truncate(stage + 1);
            let source = Fixture::start(replies).await;
            let expected = if stage < 2 {
                ProbeError::HandshakeFailed
            } else {
                ProbeError::ToolsUnavailable
            };
            assert_eq!(source.probe().await, Err(expected));
            let requests = source.finish().await;
            assert_eq!(requests.len(), stage + 1);
            assert!(
                requests
                    .iter()
                    .all(|request| !request.contains("/redirected"))
            );
            assert!(requests[0].contains("fixture-secret"));
            assert_eq!(target.finish().await, Vec::<String>::new());
        }
    }
}

/// An absent Content-Length must not bypass the streaming limit, for JSON or SSE.
#[tokio::test]
async fn rejects_oversized_chunked_responses_before_parsing_them() {
    for content_type in ["application/json", "text/event-stream"] {
        for stage in [0, 2] {
            let body = "x".repeat(MAX_RESPONSE_BYTES + 1);
            let chunked = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n",
                body.len()
            );
            let mut replies = success();
            replies[stage] = chunked;
            replies.truncate(stage + 1);
            let fixture = Fixture::start(replies).await;
            let expected = if stage == 0 {
                ProbeError::HandshakeFailed
            } else {
                ProbeError::ToolsUnavailable
            };
            assert_eq!(fixture.probe().await, Err(expected));
            assert_eq!(fixture.finish().await.len(), stage + 1);
        }
    }
}

/// A valid response exactly at the boundary is accepted; known oversize lengths are rejected.
#[tokio::test]
async fn response_limit_accepts_exactly_one_mib() {
    for extra in [0, 1] {
        let mut body = r#"{"id":1,"result":{}}"#.to_owned();
        body.push_str(&" ".repeat(MAX_RESPONSE_BYTES + extra - body.len()));
        let mut replies = success();
        replies[0] = response(
            "200 OK",
            "application/json",
            &body,
            "Mcp-Session-Id: test-session\r\n",
        );
        if extra != 0 {
            replies.truncate(1);
        }
        let fixture = Fixture::start(replies).await;
        assert_eq!(
            fixture.probe().await,
            if extra == 0 {
                Ok(())
            } else {
                Err(ProbeError::HandshakeFailed)
            }
        );
        assert_eq!(fixture.finish().await.len(), if extra == 0 { 4 } else { 1 });
    }
}

/// Proxy environments are injected only into a child test process, never mutated globally.
#[tokio::test]
async fn ignores_uppercase_and_lowercase_system_proxy_environment() {
    let fixture = Fixture::start(success()).await;
    let proxy = Fixture::start(Vec::new()).await;
    let mut command = tokio::process::Command::new(std::env::current_exe().expect("test binary"));
    command
        .args([
            "--exact",
            "mcp::tests::bounds::proxy_probe_child",
            "--nocapture",
        ])
        .env("ORA_MCP_PROXY_TEST_ENDPOINT", fixture.url.as_str())
        .env("NO_PROXY", "")
        .env("no_proxy", "");
    for name in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        command.env(name, proxy.url.as_str());
    }
    let output = command.output().await.expect("child");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(fixture.finish().await.len(), 4);
    assert_eq!(proxy.finish().await, Vec::<String>::new());
}

/// Called by the parent test with isolated proxy variables; otherwise a no-op.
#[tokio::test]
async fn proxy_probe_child() {
    let Ok(endpoint) = std::env::var("ORA_MCP_PROXY_TEST_ENDPOINT") else {
        return;
    };
    assert_eq!(
        probe(
            ProbeTransport::Http {
                url: Url::parse(&endpoint).expect("url"),
                headers: Vec::new(),
            },
            Duration::from_secs(5)
        )
        .await,
        Ok(())
    );
}

/// Unterminated frames and many individually small noise lines share the same read budget.
#[tokio::test]
async fn stdio_bounds_lines_noise_and_the_whole_handshake() {
    for (mode, expected) in [
        ("oversized-line", ProbeError::HandshakeFailed),
        ("oversized-noise", ProbeError::HandshakeFailed),
        ("oversized-tools", ProbeError::ToolsUnavailable),
    ] {
        assert_eq!(
            probe(super::fake_server_transport(mode), Duration::from_secs(5)).await,
            Err(expected)
        );
    }
}
