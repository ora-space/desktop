use super::*;
use pretty_assertions::assert_eq;
use rusqlite::OptionalExtension;

#[test]
fn probe_budget_reports_timeout_once_while_original_stop_bound_retains_capacity() {
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
        let mut child = launch(&fixture, &config);
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
            assert_eq!(state, "stopping", "10-second probe budget must not replace the D5 stop bound");
            tokio::time::sleep(std::time::Duration::from_millis(350)).await;
            send(&mut stream, get()).await;
            assert_eq!(reply(&mut stream).await.state.unwrap().state_version, timed_out.state_version, "no repeated timeout state writes");
            fs::write(&lock, original).unwrap();
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
