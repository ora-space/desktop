//! SessionHost -> plugin transport -> real ACP `session/new`: host MCP servers follow the plugin
//! servers, need HTTP MCP, and their URL and headers reach nothing but the Agent.
use super::mcp::{configure, input};
use super::support::*;
use ora_node::{SessionHost, SessionWorkload};
use ora_node_db::{NodeDatabase, NodeIdentity, SessionCommandInput};
use ora_node_protocol::*;
use pretty_assertions::assert_eq;
use std::path::Path;

/// Mirrors the echo agent's expected host input; it must never reach a file, log or event.
const SECRET: &str = "m1264-host-secret";

/// The two host servers the echo agent verifies, the first pointing at `port`.
fn host_servers(port: u16) -> Vec<HostMcpServer> {
    vec![
        HostMcpServer {
            name: "ora-alpha".into(),
            url: format!("http://127.0.0.1:{port}/mcp/{SECRET}"),
            headers: [("authorization".to_owned(), format!("Bearer {SECRET}"))]
                .into_iter()
                .collect(),
        },
        HostMcpServer {
            name: "ora-beta".into(),
            url: "http://[::1]:41235/mcp".into(),
            headers: HostMcpHeaders::default(),
        },
    ]
}

/// A loopback listener the Node must never connect to; its port is the host URL's.
fn host_listener(fixture: &Fixture) -> std::net::TcpListener {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    std::fs::write(
        fixture.package_root().join("host-mcp-port"),
        listener.local_addr().unwrap().port().to_string(),
    )
    .unwrap();
    listener
}

/// Proves nobody, the Node included, opened a connection to the host URL.
fn assert_never_connected(listener: &std::net::TcpListener) {
    assert_eq!(
        listener.accept().map_err(|error| error.kind()).err(),
        Some(std::io::ErrorKind::WouldBlock)
    );
}

/// What the echo agent recorded for each setup request it received.
fn host_frames(fixture: &Fixture) -> Vec<String> {
    std::fs::read_to_string(fixture.package_root().join("echo-host-mcp"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Searches every file under `path`, including binary ones, for the host secret.
fn assert_no_secret_files(path: &Path) {
    for entry in std::fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            assert_no_secret_files(&path);
        } else {
            let bytes = std::fs::read(&path).unwrap();
            assert!(
                !bytes
                    .windows(SECRET.len())
                    .any(|bytes| bytes == SECRET.as_bytes()),
                "host secret persisted at {}",
                path.display()
            );
        }
    }
}

/// Runs `body` on a current-thread runtime with every log line recorded to the returned text.
fn with_recorded_logs(fixture: &Fixture, body: impl AsyncFnOnce()) -> String {
    let log_path = fixture.home().join("host-mcp-trace.log");
    let log = std::fs::File::create(&log_path).unwrap();
    ora_logging::with_recorded_trace_logging(
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(move || log.try_clone().unwrap()),
        || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(body());
        },
    );
    std::fs::read_to_string(log_path).unwrap()
}

/// With plugin MCP the host servers follow both plugin servers unchanged; with an empty `mcp`
/// only the host servers are sent, and an Agent without `session/load` still starts. Neither the
/// ledger, the history, the logs nor the Thread ever holds the URL or a header value.
#[test]
fn host_servers_reach_session_new_after_plugin_servers_without_persisting() {
    for (with_plugins, load) in [(true, true), (false, false)] {
        let fixture = Fixture::new();
        let listener = host_listener(&fixture);
        let logs = with_recorded_logs(&fixture, async || {
            let bindings = configure(&fixture, load, /*http*/ true);
            let mut database = NodeDatabase::open(
                &fixture.home().join("ledger"),
                NodeIdentity::Require(NodeId::new("node-1")),
            )
            .unwrap();
            let mut start = input(if with_plugins { bindings } else { Vec::new() });
            start.payload.spec.host_mcp_servers =
                host_servers(listener.local_addr().unwrap().port());
            database.accept_session(&start).unwrap();
            database
                .start_session(&start, &fixture.node().incarnation_id)
                .unwrap();
            let sessions = fixture.compose_with_ledger(
                PLUGIN_VERSION,
                SessionWorkload::Shared,
                /*model_proxy*/ None,
                database.session_journal().unwrap(),
            );
            sessions.start(
                start.operation_id.clone(),
                start.execution_id.clone(),
                start.payload.spec.clone(),
            );
            until(|| {
                database
                    .pending_events()
                    .unwrap()
                    .iter()
                    .any(|event| {
                        matches!(event, NodeToControllerMessage::ThreadEvent(message)
                        if super::field(&message.payload.record, &["type"]) == "turnEnded")
                    })
                    .then_some(())
            })
            .await;
            let events = serde_json::to_string(&database.pending_events().unwrap()).unwrap();
            assert_eq!(events.contains("MCP frame verified"), with_plugins);
            assert!(!events.contains(SECRET));
            database
                .accept_session_command(&SessionCommandInput::EndSession(EndSessionMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    operation_id: start.operation_id.clone(),
                    execution_id: start.execution_id.clone(),
                    payload: EndSession {
                        node_id: NodeId::new("node-1"),
                        command_id: CommandId::new("end"),
                        reason: EndSessionReason::UserEnded,
                    },
                }))
                .unwrap();
            sessions.command_arrived(&start.execution_id);
            let ended = until(|| {
                match database
                    .find_session(&start.operation_id, &start.execution_id)
                    .unwrap()
                    .unwrap()
                    .state
                {
                    ExecutionState::Completed(ExecutionResult::AgentSession(
                        AgentSessionResult::AgentSessionEnded(ended),
                    )) => Some(ended),
                    _ => None,
                }
            })
            .await;
            assert_eq!(ended.reason, AgentSessionEndReason::UserEnded);
            sessions.shutdown().await;
        });
        let plugins = if with_plugins { 2 } else { 0 };
        assert_eq!(
            host_frames(&fixture),
            vec![format!("session/new plugins={plugins} host=verified")]
        );
        assert!(logs.contains("session/new"));
        assert!(!logs.contains(SECRET));
        assert_no_secret_files(&fixture.home());
        assert_never_connected(&listener);
    }
}

/// Host servers need HTTP MCP: without it the session ends after `initialize` and before any
/// `session/new`, with the host code alone; a plugin needing HTTP keeps its own code and identity.
#[test]
fn host_capability_failure_stops_before_session_new() {
    for (with_plugins, code, plugin_id, transport) in [
        (false, "host_mcp_http_capability_missing", None, None),
        (
            true,
            "mcp_http_capability_missing",
            Some(PluginId::new("official/acme.web")),
            Some(SessionMcpTransport::Http),
        ),
    ] {
        let fixture = Fixture::new();
        let listener = host_listener(&fixture);
        let mut ended = None;
        let logs = with_recorded_logs(&fixture, async || {
            let bindings = configure(&fixture, /*load*/ true, /*http*/ false);
            let sessions = fixture.sessions(PLUGIN_VERSION);
            let mut start = input(if with_plugins { bindings } else { Vec::new() });
            start.payload.spec.host_mcp_servers =
                host_servers(listener.local_addr().unwrap().port());
            sessions.start(start.operation_id, start.execution_id, start.payload.spec);
            ended = Some(fixture.ended().await);
            sessions.shutdown().await;
        });
        let ended = ended.unwrap();
        assert_eq!(
            ended,
            AgentSessionEnded {
                node: fixture.node(),
                reason: AgentSessionEndReason::AgentFailed,
                detail: Some(code.into()),
                plugin_id: plugin_id.map(Box::new),
                setting_id: None,
                transport,
                scheduled_result: None,
                artifact_manifest: None,
            }
        );
        assert!(!serde_json::to_string(&ended).unwrap().contains(SECRET));
        assert_eq!(
            std::fs::read_to_string(fixture.package_root().join("echo-acp-methods")).unwrap(),
            "initialize\n"
        );
        assert_eq!(host_frames(&fixture), Vec::<String>::new());
        assert!(logs.contains(code));
        assert!(!logs.contains(SECRET));
        assert_no_secret_files(&fixture.home());
        assert_never_connected(&listener);
    }
}

/// Host servers never widen the setup window: an Agent that never answers a host-only
/// `session/new` fails the start once the ordinary 30-second window passes, long before the
/// 120-second window plugin MCP servers would select.
#[test]
fn host_only_setup_keeps_the_ordinary_window() {
    let fixture = Fixture::new();
    let listener = host_listener(&fixture);
    configure(&fixture, /*load*/ false, /*http*/ true);
    std::fs::write(fixture.package_root().join("stall-new"), b"").unwrap();
    let (ended, elapsed) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let sessions = fixture.sessions(PLUGIN_VERSION);
            let mut start = input(Vec::new());
            start.payload.spec.host_mcp_servers =
                host_servers(listener.local_addr().unwrap().port());
            let began = std::time::Instant::now();
            sessions.start(start.operation_id, start.execution_id, start.payload.spec);
            let ended = until_within(std::time::Duration::from_secs(110), || {
                fixture.ledger.ended()
            })
            .await;
            let elapsed = began.elapsed();
            sessions.shutdown().await;
            (ended, elapsed)
        });
    assert_eq!(
        (ended.reason, ended.detail.as_deref()),
        (
            AgentSessionEndReason::AgentFailed,
            Some("agent_start_failed")
        )
    );
    assert!(elapsed >= std::time::Duration::from_secs(30), "{elapsed:?}");
    assert!(elapsed < std::time::Duration::from_secs(110), "{elapsed:?}");
    assert_eq!(
        host_frames(&fixture),
        vec!["session/new plugins=0 host=verified".to_owned()]
    );
    assert_never_connected(&listener);
}

/// Polls `probe` for up to `bound`, which a timeout here must exceed but the MCP window must not.
async fn until_within<T>(bound: std::time::Duration, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = std::time::Instant::now() + bound;
    loop {
        if let Some(value) = probe() {
            return value;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the start did not fail within {bound:?}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}
