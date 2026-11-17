use super::*;
use pretty_assertions::assert_eq;
use rusqlite::OptionalExtension;

#[test]
fn probe_budget_reports_timeout_once_while_original_stop_bound_retains_capacity() {
    exercise_cleanup(Cleanup::Budget);
}

#[test]
fn automatic_confirmation_after_d5_releases_the_probe_lease() {
    exercise_cleanup(Cleanup::Automatic);
}

/// Covers both the health budget and the later durable ownership boundary.
fn exercise_cleanup(mode: Cleanup) {
    let past_bound = !matches!(mode, Cleanup::Budget);
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
        let script = fixture
            .config()
            .home_directory
            .join("plugins/installed/official/fixture.mcp/1.0.0/assets/server.py");
        let source = fs::read_to_string(&script).unwrap().replace(
            "import sys,json,subprocess,os",
            "import sys,json,subprocess,os,time\ntime.sleep(1)",
        );
        fs::write(script, source).unwrap();
        let mut child =
            launch_with_stop_bound(&fixture, &config, if past_bound { 500 } else { 600_000 });
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            let (mut stream, _) = connect(&fixture).await;
            let mut input = start("hello");
            input.payload.spec.mcp_lifecycle = true;
            input.payload.spec.mcp = vec![SessionMcpBinding {
                plugin_id: PluginId::new("official/fixture.mcp"), plugin_version: PluginVersion::new("1.0.0"),
                settings: [("token".into(), SettingValue::String("session-probe-secret".into()))].into()
            }];
            send(&mut stream, ControllerToNodeMessage::StartAgentSession(input)).await;
            let db = rusqlite::Connection::open(fixture.config().home_directory.join("ora-node.sqlite3")).unwrap();
            let attempt: String = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let found: Option<String> = db.query_row("SELECT attempt FROM session_stop_attempts WHERE json_extract(record,'$.kind')='stdio_probe'", [], |row| row.get(0)).optional().unwrap();
                    if let Some(attempt) = found { break attempt; }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            }).await.unwrap();
            let lock = fixture.config().home_directory.join("session-attempts").join(format!("{attempt}.lock"));
            let original = fs::read(&lock).unwrap();
            fs::write(&lock, b"unconfirmed proof").unwrap();
            let timed_out = tokio::time::timeout(std::time::Duration::from_secs(12), async {
                loop {
                    send(&mut stream, get()).await;
                    let state = reply(&mut stream).await.state.unwrap();
                    if state.members[0].cleanup_pending { break state; }
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            }).await.unwrap();
            assert_eq!(timed_out.members[0].health, SessionMcpHealth::Unhealthy { code: SessionMcpProbeErrorCode::McpProbeTimeout });
            let state: String = db.query_row("SELECT state FROM session_stop_attempts WHERE attempt=?1", [&attempt], |row| row.get(0)).unwrap();
            assert_eq!(state, if past_bound { "stop_unconfirmed" } else { "stopping" });
            tokio::time::sleep(std::time::Duration::from_millis(350)).await;
            send(&mut stream, get()).await;
            assert_eq!(reply(&mut stream).await.state.unwrap().state_version, timed_out.state_version, "no repeated timeout state writes");
            if matches!(mode, Cleanup::Root) {
                let summary = root_request(&fixture, serde_json::json!({"operation":"probe_capacity"})).await;
                assert_eq!(summary["available"], 7);
                assert_eq!(summary["residues"][0]["execution"], EXECUTION);
                assert_eq!(summary["residues"][0]["attempt"], attempt);
                assert_eq!(summary["residues"][0]["state"], "stop_unconfirmed");
            }
            fs::write(&lock, original).unwrap();
            if matches!(mode, Cleanup::Root) {
                let result = root_request(&fixture, serde_json::json!({"request_id":"recheck", "execution":EXECUTION,"attempt":attempt,"reason":"probe_capacity","mode":"recheck"})).await;
                assert_eq!(result["result"]["after"], "confirmed");
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    loop {
                        let summary = root_request(&fixture, serde_json::json!({"operation":"probe_capacity"})).await;
                        if summary["available"] == 8 { assert_eq!(summary["residues"], serde_json::json!([])); break; }
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    }
                }).await.unwrap();
            }
            if past_bound {
                send(&mut stream, end()).await;
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    while !matches!(receive(&mut stream).await, NodeToControllerMessage::AgentSessionEnded(_)) {}
                }).await.unwrap();
                // Removal observes the in-memory lease, not merely the durable cleanup flag.
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    for n in 0.. {
                        send(&mut stream, ControllerToNodeMessage::RemovePlugins(RemovePluginsMessage {
                            protocol_version: CURRENT_PROTOCOL_VERSION,
                            operation_id: OperationId::new(format!("remove-{n}")), execution_id: ExecutionId::new(format!("remove-{n}")),
                            payload: RemovePlugins { spec: RemovePluginsSpec { node_id: NodeId::new("test-node"), plugins: vec![PluginRemoval { plugin_id: PluginId::new("official/fixture.mcp"), version: PluginVersion::new("1.0.0") }] } }
                        })).await;
                        loop { if let NodeToControllerMessage::PluginsResult(result) = receive(&mut stream).await {
                            if !serde_json::to_string(&result).unwrap().contains("plugin_in_use") { return; }
                            break;
                        }}
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    }
                }).await.unwrap();
                return;
            }
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    send(&mut stream, get()).await;
                    let state = reply(&mut stream).await.state.unwrap();
                    if !state.members[0].cleanup_pending {
                        assert_eq!(state.members[0].health, timed_out.members[0].health); break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            }).await.unwrap();
            send(&mut stream, end()).await;
            while !matches!(receive(&mut stream).await, NodeToControllerMessage::AgentSessionEnded(_)) {}
        });
        child.terminate();
    });
}

#[derive(Clone, Copy)]
enum Cleanup {
    Budget,
    Automatic,
    Root,
}

#[test]
#[ignore = "requires a real root peer for the management socket"]
fn root_reconciliation_wakes_probe_owner_and_lists_named_residues() {
    assert_eq!(unsafe { libc::geteuid() }, 0);
    exercise_cleanup(Cleanup::Root);
}

/// Uses the production Unix socket, so both peer authorization and serialization are exercised.
async fn root_request(fixture: &Fixture, request: serde_json::Value) -> serde_json::Value {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let mut socket = UnixStream::connect(fixture.config().home_directory.join("session-stop.sock"))
        .await
        .unwrap();
    socket
        .write_all(format!("{request}\n").as_bytes())
        .await
        .unwrap();
    let mut response = String::new();
    BufReader::new(socket)
        .read_line(&mut response)
        .await
        .unwrap();
    serde_json::from_str(&response).unwrap()
}
