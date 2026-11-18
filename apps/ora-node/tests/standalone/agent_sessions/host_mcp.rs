//! The real service: host MCP servers are advertised, refused as a protocol violation when
//! malformed, and attached unchanged to every ACP setup through refresh and Agent replacement.
use super::mcp_lifecycle::{current_healthy, install_mcp, reply};
use super::*;
use pretty_assertions::assert_eq;
use std::path::Path;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;

/// The echo agent's expected host secret; it must never reach a file or log.
const SECRET: &str = "m1264-host-secret";

/// The two host servers the echo agent verifies, the first pointing at `port`.
fn host_servers(port: u16, secret: &str) -> Vec<HostMcpServer> {
    vec![
        HostMcpServer {
            name: "ora-alpha".into(),
            url: format!("http://127.0.0.1:{port}/mcp/{secret}"),
            headers: [("authorization".to_owned(), format!("Bearer {secret}"))]
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

/// A loopback listener nobody may connect to, whose port the echo agent expects in the URL.
fn host_listener(root: &Path) -> std::net::TcpListener {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    fs::write(
        root.join("host-mcp-port"),
        listener.local_addr().unwrap().port().to_string(),
    )
    .unwrap();
    listener
}

/// Searches every file under `path`, including the database and its WAL, for `secret`.
fn assert_absent(path: &Path, secret: &str) {
    for entry in fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            assert_absent(&path, secret);
        } else if let Ok(bytes) = fs::read(&path) {
            assert!(
                !bytes
                    .windows(secret.len())
                    .any(|window| window == secret.as_bytes()),
                "host value persisted at {}",
                path.display()
            );
        }
    }
}

/// A lifecycle session with one stdio plugin MCP and the two host servers.
fn lifecycle_start(port: u16, secret: &str) -> StartAgentSessionMessage {
    let mut input = start("hello");
    input.payload.spec.mcp_lifecycle = true;
    input.payload.spec.mcp = vec![SessionMcpBinding {
        plugin_id: PluginId::new("official/fixture.mcp"),
        plugin_version: PluginVersion::new("1.0.0"),
        settings: [(
            "token".to_string(),
            SettingValue::String("session-probe-secret".into()),
        )]
        .into(),
    }];
    input.payload.spec.host_mcp_servers = host_servers(port, secret);
    input
}

/// Moves the plugin MCP to 2.0.0, which replaces the session's Agent process.
fn version_update() -> ControllerToNodeMessage {
    ControllerToNodeMessage::UpdateSessionMcp(UpdateSessionMcpMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: start("").operation_id,
        execution_id: ExecutionId::new(EXECUTION),
        payload: UpdateSessionMcp {
            node_id: NodeId::new("test-node"),
            request_id: RequestId::new("version-update"),
            generation: 1,
            members: vec![SessionMcpUpdateMember::Configured {
                plugin_id: PluginId::new("official/fixture.mcp"),
                plugin_version: PluginVersion::new("2.0.0"),
                configuration_revision: 1,
                settings: [(
                    "token".to_string(),
                    SettingValue::String("session-probe-secret".into()),
                )]
                .into(),
            }],
        },
    })
}

/// Waits for the terminal result, skipping everything else the session streams.
async fn ended(stream: &mut UnixStream) -> AgentSessionEnded {
    loop {
        if let NodeToControllerMessage::AgentSessionEnded(event) = receive(stream).await {
            let AgentSessionResult::AgentSessionEnded(ended) = event.payload;
            return ended;
        }
    }
}

/// What the echo agent recorded for each `session/new` and `session/load` it received.
fn host_frames(root: &Path) -> Vec<String> {
    fs::read_to_string(root.join("echo-host-mcp"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Malformed host servers close the control connection before admission: no execution is
/// recorded, no Agent starts, and neither the grammar nor the decoder failure logs the value.
#[test]
fn invalid_host_servers_close_the_connection_before_admission() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let root = prepare(&fixture, &config, &server);
        let mut child = launch(&fixture, &config);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                for servers in [
                    // Outside the grammar: plaintext http on a host name.
                    serde_json::json!([{"name": "ora-alpha", "url": format!("http://localhost:1/{SECRET}"), "headers": {"authorization": SECRET}}]),
                    // A decoder message would quote this string.
                    serde_json::json!([{"name": "ora-alpha", "url": "http://127.0.0.1:1/", "headers": SECRET}]),
                ] {
                    let (mut stream, _) = connect(&fixture).await;
                    let mut wire = serde_json::to_value(ControllerToNodeMessage::StartAgentSession(
                        start("hello"),
                    ))
                    .unwrap();
                    wire["payload"]["spec"]["host_mcp_servers"] = servers;
                    let payload = serde_json::to_vec(&wire).unwrap();
                    let mut frame = u32::try_from(payload.len() + 1).unwrap().to_be_bytes().to_vec();
                    frame.push(NODE_MESSAGE_FRAME_TYPE);
                    frame.extend_from_slice(&payload);
                    stream.write_all(&frame).await.unwrap();
                    tokio::time::timeout(std::time::Duration::from_secs(60), async {
                        loop {
                            match read_node_message(&mut stream).await {
                                Ok(Some(NodeToControllerMessage::Heartbeat(_))) => {}
                                Ok(Some(other)) => panic!("Node answered a violation: {other:?}"),
                                Ok(None) | Err(_) => break,
                            }
                        }
                    })
                    .await
                    .expect("the Node closes the connection");
                }
                let (mut stream, _) = connect(&fixture).await;
                send(&mut stream, query()).await;
                loop {
                    if let NodeToControllerMessage::ExecutionStatus(status) = receive(&mut stream).await {
                        assert_eq!(status.payload.state, ExecutionState::Unknown);
                        break;
                    }
                }
            });
        child.terminate();
        assert!(!root.join("echo-agent.pids").exists());
        let log = fs::read_to_string(fixture.path().join("agent-service.log")).unwrap();
        assert!(
            log.contains("invalid payload.spec.host_mcp_servers"),
            "{log}"
        );
        assert!(
            log.contains("failed to decode Controller–Node message JSON"),
            "{log}"
        );
        assert!(!log.contains(SECRET));
        assert_absent(&fixture.config().home_directory, SECRET);
    });
}

/// The first `session/new`, the refresh load after the update and every setup of the
/// replacement Agent carry exactly the first accepted host servers after the plugin server; a
/// repeated start with other values changes nothing.
#[test]
fn host_servers_follow_every_setup_through_refresh_and_replacement() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let root = prepare(&fixture, &config, &server);
        fs::write(
            root.join("mcp-capabilities.json"),
            r#"{"load":true,"http":true}"#,
        )
        .unwrap();
        install_mcp(&fixture, "1.0.0");
        install_mcp(&fixture, "2.0.0");
        let listener = host_listener(&root);
        let port = listener.local_addr().unwrap().port();
        let mut child = launch(&fixture, &config);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (mut stream, _) = connect(&fixture).await;
                send(
                    &mut stream,
                    ControllerToNodeMessage::StartAgentSession(lifecycle_start(port, SECRET)),
                )
                .await;
                current_healthy(&mut stream, 0, 0).await;
                // Same names, other values: ignored, never a replacement of the values in use.
                send(
                    &mut stream,
                    ControllerToNodeMessage::StartAgentSession(lifecycle_start(
                        port + 1,
                        "m1264-resent",
                    )),
                )
                .await;
                send(&mut stream, version_update()).await;
                assert_eq!(reply(&mut stream).await.result, SessionMcpResult::Accepted);
                current_healthy(&mut stream, 1, 0).await;
                send(&mut stream, end()).await;
                assert_eq!(
                    ended(&mut stream).await.reason,
                    AgentSessionEndReason::UserEnded
                );
            });
        child.terminate();
        assert_eq!(
            fs::read_to_string(root.join("echo-agent.pids"))
                .unwrap()
                .lines()
                .count(),
            2,
            "the update replaced the Agent"
        );
        let frames = host_frames(&root);
        assert!(
            frames.iter().any(|frame| frame.starts_with("session/new")),
            "{frames:?}"
        );
        assert!(
            frames.iter().any(|frame| frame.starts_with("session/load")),
            "{frames:?}"
        );
        assert!(
            frames
                .iter()
                .all(|frame| frame.ends_with("plugins=1 host=verified")),
            "{frames:?}"
        );
        assert_eq!(
            listener.accept().map_err(|error| error.kind()).err(),
            Some(std::io::ErrorKind::WouldBlock)
        );
        let log = fs::read_to_string(fixture.path().join("agent-service.log")).unwrap();
        assert!(!log.contains(SECRET) && !log.contains("m1264-resent"));
        assert_absent(&fixture.config().home_directory, SECRET);
        assert_absent(&fixture.config().home_directory, "m1264-resent");
    });
}

/// A replacement Agent that no longer declares HTTP MCP ends the execution with the host code,
/// without ever receiving the host servers on its connection.
#[test]
fn replacement_without_http_mcp_ends_with_the_host_code() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let root = prepare(&fixture, &config, &server);
        fs::write(
            root.join("mcp-capabilities.json"),
            r#"{"load":true,"http":true}"#,
        )
        .unwrap();
        install_mcp(&fixture, "1.0.0");
        install_mcp(&fixture, "2.0.0");
        let listener = host_listener(&root);
        let port = listener.local_addr().unwrap().port();
        let mut child = launch(&fixture, &config);
        let ended = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (mut stream, _) = connect(&fixture).await;
                send(
                    &mut stream,
                    ControllerToNodeMessage::StartAgentSession(lifecycle_start(port, SECRET)),
                )
                .await;
                current_healthy(&mut stream, 0, 0).await;
                let before = host_frames(&root);
                fs::write(
                    root.join("mcp-capabilities.json"),
                    r#"{"load":true,"http":false}"#,
                )
                .unwrap();
                send(&mut stream, version_update()).await;
                assert_eq!(reply(&mut stream).await.result, SessionMcpResult::Accepted);
                (before, ended(&mut stream).await)
            });
        child.terminate();
        let (before, ended) = ended;
        assert_eq!(
            (
                ended.reason,
                ended.detail.as_deref(),
                ended.plugin_id,
                ended.transport
            ),
            (
                AgentSessionEndReason::AgentFailed,
                Some("host_mcp_http_capability_missing"),
                None,
                None
            )
        );
        assert_eq!(
            host_frames(&root),
            before,
            "the replacement received no setup"
        );
        assert_eq!(
            fs::read_to_string(root.join("echo-agent.pids"))
                .unwrap()
                .lines()
                .count(),
            2
        );
        let log = fs::read_to_string(fixture.path().join("agent-service.log")).unwrap();
        assert!(log.contains("host_mcp_http_capability_missing"));
        assert!(!log.contains(SECRET));
        assert_absent(&fixture.config().home_directory, SECRET);
    });
}
