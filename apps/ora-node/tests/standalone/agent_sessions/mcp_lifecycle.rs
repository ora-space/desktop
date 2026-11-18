//! Real framed updates and repeated loads cross the process budget without replacing the session.
use super::*;
use pretty_assertions::assert_eq;
use tokio::net::UnixStream;
#[path = "mcp_lifecycle/cleanup.rs"]
mod cleanup;
#[path = "mcp_lifecycle/races.rs"]
mod races;

#[test]
fn lifecycle_agent_failure_is_terminal_and_unknown_execution_keeps_connection_open() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server = GitServerFixture::new(fixture.path(), fixture.path().join("main/.git"));
        let config = configuration(&fixture, &server);
        let root = prepare(&fixture, &config, &server);
        fs::write(root.join("fail-new"), b"").unwrap();
        let mut child = launch(&fixture, &config);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (mut stream, _) = connect(&fixture).await;
                send(&mut stream, get()).await;
                assert_eq!(reply(&mut stream).await.result, SessionMcpResult::NotFound);
                let mut input = start("hello");
                input.payload.spec.mcp_lifecycle = true;
                send(
                    &mut stream,
                    ControllerToNodeMessage::StartAgentSession(input),
                )
                .await;
                tokio::time::timeout(std::time::Duration::from_secs(15), async {
                    loop {
                        match receive(&mut stream).await {
                            NodeToControllerMessage::AgentSessionEnded(event) => {
                                let AgentSessionResult::AgentSessionEnded(ended) = event.payload;
                                assert_eq!(ended.detail.as_deref(), Some("agent_unavailable"));
                                break;
                            }
                            NodeToControllerMessage::SessionTurnFailed(_) => {
                                panic!("Agent failure is not MCP timeout")
                            }
                            _ => {}
                        }
                    }
                })
                .await
                .unwrap();
            });
        child.terminate();
    });
}

pub(super) async fn reply(stream: &mut UnixStream) -> SessionMcpReply {
    loop {
        match receive(stream).await {
            NodeToControllerMessage::SessionMcpReply(reply) => return reply.payload,
            NodeToControllerMessage::ThreadEvent(_)
            | NodeToControllerMessage::ExecutionStatus(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
}

pub(super) fn get() -> ControllerToNodeMessage {
    ControllerToNodeMessage::GetSessionMcpState(GetSessionMcpStateMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: start("").operation_id,
        execution_id: ExecutionId::new(EXECUTION),
        payload: GetSessionMcpState {
            node_id: NodeId::new("test-node"),
            request_id: RequestId::new("get"),
        },
    })
}

/// A real stdio server leaves a descendant so Healthy requires process-group cleanup as well.
pub(super) fn install_mcp(fixture: &Fixture, version: &str) {
    let root = fixture
        .config()
        .home_directory
        .join("plugins/installed/official/fixture.mcp")
        .join(version);
    fs::create_dir_all(root.join("assets")).unwrap();
    fs::write(root.join("orax.toml"),format!("resolver = 1\nidentifier = \"fixture.mcp\"\nkind = \"mcp\"\nversion = \"{version}\"\ndescription = \"test\"\n")).unwrap();
    fs::write(root.join("assets/config.json"),r#"{"schemaVersion":1,"settings":{"token":{"type":"string","title":"Token","description":"test","required":true}},"transport":{"type":"stdio","command":"assets/server.py","env":{"TOKEN":{"setting":"token"}}}}"#).unwrap();
    let executable = root.join("assets/server.py");
    fs::write(&executable,r#"#!/usr/bin/python3
import sys,json,subprocess,os
assert os.environ['TOKEN'] == 'session-probe-secret'
assert os.environ.get('PATH'), 'stdio probes require the essential process environment'
child = subprocess.Popen(['/bin/sleep','60'])
with open('.mcp-descendants','a') as output: output.write(str(child.pid)+'\n')
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request: continue
    result = {'tools':[]} if request['method']=='tools/list' else {'protocolVersion':'2024-11-05','capabilities':{'tools':{}},'serverInfo':{'name':'fixture','version':'1'}}
    print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':result}),flush=True)
"#).unwrap();
    fs::set_permissions(executable, fs::Permissions::from_mode(0o755)).unwrap();
}

pub(super) async fn current_healthy(stream: &mut UnixStream, generation: u64, attempt: u64) {
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            send(stream, get()).await;
            let state = reply(stream).await.state.unwrap();
            if state.convergence == SessionMcpConvergence::Current
                && state.desired.generation == generation
                && state.members.iter().all(|member| {
                    member.health == SessionMcpHealth::Healthy
                        && member.probe_attempt == attempt
                        && !member.queued
                        && !member.probe_in_flight
                })
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("host handshake and cleanup must converge");
}

/// The production clock is intentionally exercised without shortening the protocol's 300 seconds.
#[test]

fn blocked_initial_and_queued_turns_fail_once_without_ending_the_session() {
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
        let mut child = launch(&fixture, &config);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (mut stream, _) = connect(&fixture).await;
                let mut input = start("blocked");
                input.payload.spec.mcp_lifecycle = true;
                input.payload.spec.mcp = vec![SessionMcpBinding {
                    plugin_id: PluginId::new("official/missing"),
                    plugin_version: PluginVersion::new("1.0.0"),
                    settings: Default::default(),
                }];
                let began = tokio::time::Instant::now();
                send(
                    &mut stream,
                    ControllerToNodeMessage::StartAgentSession(input),
                )
                .await;
                for index in 0..2 {
                    let mut command = turn();
                    if let ControllerToNodeMessage::SubmitUserTurn(input) = &mut command {
                        input.payload.command_id = CommandId::new(format!("queued-{index}"));
                        input.payload.turn.turn_id = TurnId::new(format!("queued-{index}"));
                        input.payload.mcp_generation = Some(0);
                    }
                    send(&mut stream, command).await;
                }
                let mut failed = std::collections::BTreeSet::new();
                tokio::time::timeout(std::time::Duration::from_secs(310), async {
                    loop {
                        send(&mut stream, get()).await;
                        loop {
                            match receive(&mut stream).await {
                                NodeToControllerMessage::SessionMcpReply(_) => break,
                                NodeToControllerMessage::SessionTurnFailed(event) => {
                                    assert!(failed.insert(event.payload.turn_id.to_string()));
                                    send(&mut stream, ack(event.sequence.value())).await;
                                }
                                NodeToControllerMessage::ExecutionStatus(_)
                                | NodeToControllerMessage::SessionCommandAccepted(_) => {}
                                other => panic!(
                                    "blocked admission must not emit Thread or Ended: {other:?}"
                                ),
                            }
                        }
                        if failed.len() == 3 {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                })
                .await
                .expect("each turn has its own bounded failure");
                assert!(began.elapsed() >= std::time::Duration::from_secs(300));
                send(&mut stream, end()).await;
                loop {
                    if matches!(
                        receive(&mut stream).await,
                        NodeToControllerMessage::AgentSessionEnded(_)
                    ) {
                        break;
                    }
                }
            });
        child.terminate();
        assert!(
            !fs::read_to_string(root.join("echo-acp-methods"))
                .unwrap_or_default()
                .lines()
                .any(|method| method == "session/new")
        );
    });
}

#[test]
fn version_update_replaces_agent_and_host_probes_reap_real_descendants() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let root = prepare(&fixture, &config, &server);
        fs::write(root.join("require-path"), b"").unwrap();
        fs::write(
            root.join("mcp-capabilities.json"),
            r#"{"load":true,"http":true}"#,
        )
        .unwrap();
        install_mcp(&fixture, "1.0.0");
        install_mcp(&fixture, "2.0.0");
        let mut child = launch(&fixture, &config);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (mut stream, _) = connect(&fixture).await;
                let mut input = start("hello");
                input.payload.spec.mcp_lifecycle = true;
                let settings = [(
                    "token".to_string(),
                    SettingValue::String("session-probe-secret".into()),
                )]
                .into();
                input.payload.spec.mcp = vec![SessionMcpBinding {
                    plugin_id: PluginId::new("official/fixture.mcp"),
                    plugin_version: PluginVersion::new("1.0.0"),
                    settings,
                }];
                let binding = input.payload.spec.mcp[0].clone();
                send(
                    &mut stream,
                    ControllerToNodeMessage::StartAgentSession(input),
                )
                .await;
                current_healthy(&mut stream, 0, 0).await;
                let db = rusqlite::Connection::open(fixture.config().home_directory.join("ora-node.sqlite3")).unwrap();
                let attempt: String = db.query_row("SELECT attempt FROM session_stop_attempts WHERE json_extract(record,'$.kind')='agent'", [], |row| row.get(0)).unwrap();
                let lock = fixture.config().home_directory.join("session-attempts").join(format!("{attempt}.lock"));
                let marker = fs::read(&lock).unwrap();
                fs::write(&lock, b"missing original proof").unwrap();
                send(
                    &mut stream,
                    ControllerToNodeMessage::UpdateSessionMcp(UpdateSessionMcpMessage {
                        protocol_version: CURRENT_PROTOCOL_VERSION,
                        operation_id: start("").operation_id,
                        execution_id: ExecutionId::new(EXECUTION),
                        payload: UpdateSessionMcp {
                            node_id: NodeId::new("test-node"),
                            request_id: RequestId::new("version-update"),
                            generation: 1,
                            members: vec![SessionMcpUpdateMember::Configured {
                                plugin_id: binding.plugin_id.clone(),
                                plugin_version: PluginVersion::new("2.0.0"),
                                configuration_revision: 1,
                                settings: binding.settings,
                            }],
                        },
                    }),
                )
                .await;
                assert_eq!(reply(&mut stream).await.result, SessionMcpResult::Accepted);
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    loop {
                        let requested: bool = db.query_row("SELECT first_stop_at_ms IS NOT NULL FROM session_stop_attempts WHERE attempt=?1", [&attempt], |row| row.get(0)).unwrap();
                        if requested { break; }
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                }).await.unwrap();
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                assert_eq!(fs::read_to_string(root.join("echo-agent.pids")).unwrap().lines().count(), 1, "no replacement before affirmative original stop proof");
                send(&mut stream, get()).await;
                assert_ne!(reply(&mut stream).await.state.unwrap().convergence, SessionMcpConvergence::Current);
                fs::write(&lock, marker).unwrap();
                current_healthy(&mut stream, 1, 0).await;
                send(
                    &mut stream,
                    ControllerToNodeMessage::ProbeSessionMcp(ProbeSessionMcpMessage {
                        protocol_version: CURRENT_PROTOCOL_VERSION,
                        operation_id: start("").operation_id,
                        execution_id: ExecutionId::new(EXECUTION),
                        payload: ProbeSessionMcp {
                            node_id: NodeId::new("test-node"),
                            request_id: RequestId::new("manual"),
                            generation: 1,
                            plugin_id: binding.plugin_id,
                            probe_attempt: 1,
                        },
                    }),
                )
                .await;
                assert_eq!(reply(&mut stream).await.result, SessionMcpResult::Accepted);
                current_healthy(&mut stream, 1, 1).await;
                send(&mut stream, end()).await;
                loop {
                    if matches!(
                        receive(&mut stream).await,
                        NodeToControllerMessage::AgentSessionEnded(_)
                    ) {
                        break;
                    }
                }
            });
        child.terminate();
        assert_eq!(
            fs::read_to_string(root.join("echo-agent.pids"))
                .unwrap()
                .lines()
                .count(),
            2
        );
        assert_eq!(
            fs::read_to_string(root.join("echo-acp-methods"))
                .unwrap()
                .lines()
                .filter(|method| *method == "session/new")
                .count(),
            1
        );
    });
}

#[test]
fn ordered_updates_floor_rejection_and_process_budget_use_the_real_service() {
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
        let mut child = launch(&fixture, &config);
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            let (mut stream,_) = connect(&fixture).await;
            let mut input = start("hello"); input.payload.spec.mcp_lifecycle = true;
            send(&mut stream,ControllerToNodeMessage::StartAgentSession(input)).await;
            loop {
                if let NodeToControllerMessage::ThreadEvent(event) = receive(&mut stream).await
                    && event.payload.record.get("type").and_then(serde_json::Value::as_str) == Some("turnEnded") { break; }
            }
            send(&mut stream,turn()).await;
            assert!(matches!(receive(&mut stream).await,NodeToControllerMessage::SessionCommandRejected(reply) if reply.payload.reason == SessionCommandRejection::McpUpdateRequired));
            for generation in 1..=10 {
                let update = ControllerToNodeMessage::UpdateSessionMcp(UpdateSessionMcpMessage { protocol_version: CURRENT_PROTOCOL_VERSION, operation_id: start("").operation_id, execution_id: ExecutionId::new(EXECUTION), payload: UpdateSessionMcp { node_id: NodeId::new("test-node"), request_id: RequestId::new(format!("update-{generation}")), generation, members: vec![] } });
                send(&mut stream,update.clone()).await;
                assert_eq!(reply(&mut stream).await.result,SessionMcpResult::Accepted);
                send(&mut stream,update).await;
                assert_eq!(reply(&mut stream).await.result,SessionMcpResult::Duplicate);
                tokio::time::timeout(std::time::Duration::from_secs(15),async {
                    loop {
                        send(&mut stream,get()).await;
                        let state = reply(&mut stream).await.state.unwrap();
                        if state.convergence == SessionMcpConvergence::Current { assert_eq!(state.active.unwrap().generation,generation); break; }
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    }
                }).await.expect("generation must converge");
                assert_eq!(fs::read_to_string(root.join("echo-agent.pids")).unwrap().lines().count(), if generation < 9 { 1 } else { 2 }, "exact process boundary at runtime load {generation}");
            }
            let mut turn = turn();
            if let ControllerToNodeMessage::SubmitUserTurn(input) = &mut turn { input.payload.mcp_generation = Some(10); }
            send(&mut stream,turn).await;
            loop {
                if let NodeToControllerMessage::ThreadEvent(event) = receive(&mut stream).await
                    && event.payload.record.get("type").and_then(serde_json::Value::as_str) == Some("turnEnded") { break; }
            }
            send(&mut stream,end()).await;
            loop { if matches!(receive(&mut stream).await,NodeToControllerMessage::AgentSessionEnded(_)) { break; } }
            send(&mut stream,get()).await;
            let ended = reply(&mut stream).await;
            assert_eq!(ended.result,SessionMcpResult::SessionEnded); assert!(ended.state.is_none());
        });
        child.terminate();
        let pids = fs::read_to_string(root.join("echo-agent.pids")).unwrap();
        assert_eq!(
            pids.lines().count(),
            2,
            "the ninth runtime load rotates only this Agent once"
        );
        assert_eq!(
            fs::read_to_string(root.join("echo-acp-methods"))
                .unwrap()
                .lines()
                .filter(|method| *method == "session/new")
                .count(),
            1
        );
    });
}
