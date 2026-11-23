use super::*;
use pretty_assertions::assert_eq;

/// Sends a new complete generation without introducing another persistent command.
fn update(generation: u64, members: Vec<SessionMcpUpdateMember>) -> ControllerToNodeMessage {
    ControllerToNodeMessage::UpdateSessionMcp(UpdateSessionMcpMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: start("").operation_id,
        execution_id: ExecutionId::new(EXECUTION),
        payload: UpdateSessionMcp {
            node_id: NodeId::new("test-node"),
            request_id: RequestId::new(format!("update-{generation}")),
            generation,
            members,
        },
    })
}

#[test]
fn normal_end_does_not_publish_terminal_until_original_stop_evidence_returns() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        prepare(&fixture, &config, &server);
        let mut child = launch(&fixture, &config);
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            let (mut stream, _) = connect(&fixture).await;
            send(&mut stream, ControllerToNodeMessage::StartAgentSession(start("hello"))).await;
            loop { if let NodeToControllerMessage::ThreadEvent(event) = receive(&mut stream).await
                && event.payload.record.get("type").and_then(serde_json::Value::as_str) == Some("turnEnded") { break; } }
            let db = rusqlite::Connection::open(fixture.config().home_directory.join("ora-node.sqlite3")).unwrap();
            let attempt: String = db.query_row("SELECT attempt FROM session_stop_attempts WHERE json_extract(record,'$.kind')='agent'", [], |row| row.get(0)).unwrap();
            let lock = fixture.config().home_directory.join("session-attempts").join(format!("{attempt}.lock"));
            let marker = fs::read(&lock).unwrap(); fs::write(&lock, b"no proof").unwrap();
            send(&mut stream, end()).await;
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while !db.query_row("SELECT first_stop_at_ms IS NOT NULL FROM session_stop_attempts WHERE attempt=?1", [&attempt], |row| row.get::<_,bool>(0)).unwrap() {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            }).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let state: String = db.query_row("SELECT state FROM node_executions WHERE execution=?1", [EXECUTION], |row| row.get(0)).unwrap();
            assert_eq!(state, "running");
            let stop_state: String = db.query_row("SELECT state FROM session_stop_attempts WHERE attempt=?1", [&attempt], |row| row.get(0)).unwrap();
            assert_eq!(stop_state, "stopping", "missing proof must still await the original bound");
            assert_eq!(db.query_row("SELECT resources_released FROM session_stop_attempts WHERE attempt=?1", [&attempt], |row| row.get::<_,i64>(0)).unwrap(), 0);
            fs::write(&lock, marker).unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while !matches!(receive(&mut stream).await, NodeToControllerMessage::AgentSessionEnded(_)) {}
            }).await.unwrap();
        });
        child.terminate();
    });
}

#[test]
fn turn_arriving_during_idle_load_waits_for_that_load_without_destroying_sending() {
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
        fs::write(root.join("load-delay-ms"), b"500").unwrap();
        let mut child = launch(&fixture, &config);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (mut stream, _) = connect(&fixture).await;
                let mut input = start("hello");
                input.payload.spec.mcp_lifecycle = true;
                send(
                    &mut stream,
                    ControllerToNodeMessage::StartAgentSession(input),
                )
                .await;
                loop {
                    if let NodeToControllerMessage::ThreadEvent(event) = receive(&mut stream).await
                        && event
                            .payload
                            .record
                            .get("type")
                            .and_then(serde_json::Value::as_str)
                            == Some("turnEnded")
                    {
                        break;
                    }
                }
                send(&mut stream, update(1, vec![])).await;
                assert_eq!(reply(&mut stream).await.result, SessionMcpResult::Accepted);
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    while !fs::read_to_string(root.join("echo-acp-methods"))
                        .unwrap()
                        .contains("session/load")
                    {
                        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                    }
                })
                .await
                .unwrap();
                let mut next = turn();
                if let ControllerToNodeMessage::SubmitUserTurn(input) = &mut next {
                    input.payload.mcp_generation = Some(1);
                }
                send(&mut stream, next).await;
                tokio::time::timeout(std::time::Duration::from_secs(10), async {
                    loop {
                        match receive(&mut stream).await {
                            NodeToControllerMessage::ThreadEvent(event)
                                if event
                                    .payload
                                    .record
                                    .get("type")
                                    .and_then(serde_json::Value::as_str)
                                    == Some("turnEnded") =>
                            {
                                break;
                            }
                            NodeToControllerMessage::AgentSessionEnded(_)
                            | NodeToControllerMessage::SessionTurnFailed(_) => {
                                panic!("busy refresh must converge")
                            }
                            _ => {}
                        }
                    }
                })
                .await
                .unwrap();
                send(&mut stream, get()).await;
                assert_eq!(
                    reply(&mut stream).await.state.unwrap().convergence,
                    SessionMcpConvergence::Current
                );
                send(&mut stream, end()).await;
                while !matches!(
                    receive(&mut stream).await,
                    NodeToControllerMessage::AgentSessionEnded(_)
                ) {}
            });
        child.terminate();
        assert_eq!(
            fs::read_to_string(root.join("echo-acp-methods"))
                .unwrap()
                .lines()
                .filter(|line| *line == "session/load")
                .count(),
            1
        );
    });
}

#[test]
fn failed_replacement_resumes_on_new_target_without_new_provider_session() {
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
        let mut child = launch(&fixture, &config);
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            let (mut stream, _) = connect(&fixture).await;
            let settings: std::collections::BTreeMap<String, SettingValue> = [("token".into(), SettingValue::String("session-probe-secret".into()))].into();
            let binding = SessionMcpBinding { plugin_id: PluginId::new("official/fixture.mcp"), plugin_version: PluginVersion::new("1.0.0"), settings };
            let mut input = start("hello"); input.payload.spec.mcp_lifecycle = true; input.payload.spec.mcp = vec![binding.clone()];
            send(&mut stream, ControllerToNodeMessage::StartAgentSession(input)).await;
            current_healthy(&mut stream, 0, 0).await;
            let db = rusqlite::Connection::open(fixture.config().home_directory.join("ora-node.sqlite3")).unwrap();
            let attempt: String = db.query_row("SELECT attempt FROM session_stop_attempts WHERE json_extract(record,'$.kind')='agent'", [], |row| row.get(0)).unwrap();
            let lock = fixture.config().home_directory.join("session-attempts").join(format!("{attempt}.lock"));
            let marker = fs::read(&lock).unwrap(); fs::write(&lock, b"wait for proof").unwrap();
            let target = SessionMcpUpdateMember::Configured { plugin_id: binding.plugin_id.clone(), plugin_version: PluginVersion::new("2.0.0"), configuration_revision: 1, settings: binding.settings };
            send(&mut stream, update(1, vec![target.clone()])).await; reply(&mut stream).await;
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while !db.query_row("SELECT first_stop_at_ms IS NOT NULL FROM session_stop_attempts WHERE attempt=?1", [&attempt], |row| row.get::<_,bool>(0)).unwrap() {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            }).await.unwrap();
            send(&mut stream, update(2, vec![SessionMcpUpdateMember::Unavailable { plugin_id: binding.plugin_id, code: SessionMcpUnavailable::McpPluginMissing }])).await;
            reply(&mut stream).await; fs::write(&lock, marker).unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop { send(&mut stream, get()).await; if reply(&mut stream).await.state.unwrap().convergence == SessionMcpConvergence::Blocked { break; }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            }).await.unwrap();
            assert_eq!(fs::read_to_string(root.join("echo-agent.pids")).unwrap().lines().count(), 1);
            send(&mut stream, update(3, vec![target])).await; reply(&mut stream).await;
            current_healthy(&mut stream, 3, 0).await;
            send(&mut stream, end()).await;
            while !matches!(receive(&mut stream).await, NodeToControllerMessage::AgentSessionEnded(_)) {}
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
                .filter(|line| *line == "session/new")
                .count(),
            1
        );
    });
}

/// Exercises the production guard, not just its clock helper, with real Agent business time.
#[test]
fn queued_turn_survives_business_time_longer_than_the_admission_budget() {
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
        fs::write(root.join("prompt-delay-ms"), b"301000").unwrap();
        install_mcp(&fixture, "1.0.0");
        let mut child = launch(&fixture, &config);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (mut stream, _) = connect(&fixture).await;
                let settings: std::collections::BTreeMap<String, SettingValue> = [(
                    "token".into(),
                    SettingValue::String("session-probe-secret".into()),
                )]
                .into();
                let mut input = start("first");
                input.payload.spec.mcp_lifecycle = true;
                input.payload.spec.mcp = vec![SessionMcpBinding {
                    plugin_id: PluginId::new("official/fixture.mcp"),
                    plugin_version: PluginVersion::new("9.0.0"),
                    settings: settings.clone(),
                }];
                send(
                    &mut stream,
                    ControllerToNodeMessage::StartAgentSession(input),
                )
                .await;
                let blocked_version = loop {
                    send(&mut stream, get()).await;
                    let state = reply(&mut stream).await.state.unwrap();
                    if state.convergence == SessionMcpConvergence::Blocked {
                        break state.state_version;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                };
                let mut queued = turn();
                if let ControllerToNodeMessage::SubmitUserTurn(input) = &mut queued {
                    input.payload.mcp_generation = Some(0);
                }
                send(&mut stream, queued).await;
                while !matches!(
                    receive(&mut stream).await,
                    NodeToControllerMessage::SessionCommandAccepted(_)
                ) {}
                // Wait for the actor to consume the queue wake while the missing target still blocks it.
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    loop {
                        send(&mut stream, get()).await;
                        if reply(&mut stream).await.state.unwrap().state_version > blocked_version {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                })
                .await
                .unwrap();
                send(
                    &mut stream,
                    update(
                        1,
                        vec![SessionMcpUpdateMember::Configured {
                            plugin_id: PluginId::new("official/fixture.mcp"),
                            plugin_version: PluginVersion::new("1.0.0"),
                            configuration_revision: 1,
                            settings,
                        }],
                    ),
                )
                .await;
                assert_eq!(reply(&mut stream).await.result, SessionMcpResult::Accepted);
                tokio::time::timeout(std::time::Duration::from_secs(330), async {
                    let mut completed = 0;
                    let mut queried = tokio::time::Instant::now();
                    while completed < 2 {
                        if queried.elapsed() >= std::time::Duration::from_secs(5) {
                            send(&mut stream, get()).await;
                            queried = tokio::time::Instant::now();
                        }
                        let message = read_node_message(&mut stream).await.unwrap().unwrap();
                        match message {
                            NodeToControllerMessage::ThreadEvent(event) => {
                                if event
                                    .payload
                                    .record
                                    .get("type")
                                    .and_then(serde_json::Value::as_str)
                                    == Some("turnEnded")
                                {
                                    completed += 1;
                                }
                            }
                            NodeToControllerMessage::SessionTurnFailed(_)
                            | NodeToControllerMessage::AgentSessionEnded(_) => {
                                panic!("queued turn lost its business-time exclusion: {message:?}")
                            }
                            _ => {}
                        }
                    }
                })
                .await
                .unwrap();
                send(&mut stream, end()).await;
                while !matches!(
                    receive(&mut stream).await,
                    NodeToControllerMessage::AgentSessionEnded(_)
                ) {}
            });
        child.terminate();
    });
}

#[test]
fn replacement_initialize_failure_is_blocked_and_cannot_auto_restart() {
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
        let mut child = launch(&fixture, &config);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (mut stream, _) = connect(&fixture).await;
                let binding = SessionMcpBinding {
                    plugin_id: PluginId::new("official/fixture.mcp"),
                    plugin_version: PluginVersion::new("1.0.0"),
                    settings: [(
                        "token".into(),
                        SettingValue::String("session-probe-secret".into()),
                    )]
                    .into(),
                };
                let mut input = start("hello");
                input.payload.spec.mcp_lifecycle = true;
                input.payload.spec.mcp = vec![binding.clone()];
                send(
                    &mut stream,
                    ControllerToNodeMessage::StartAgentSession(input),
                )
                .await;
                current_healthy(&mut stream, 0, 0).await;
                fs::write(root.join("fail-initialize"), b"").unwrap();
                let target = SessionMcpUpdateMember::Configured {
                    plugin_id: binding.plugin_id,
                    plugin_version: PluginVersion::new("2.0.0"),
                    configuration_revision: 1,
                    settings: binding.settings,
                };
                send(&mut stream, update(1, vec![target.clone()])).await;
                reply(&mut stream).await;
                tokio::time::timeout(std::time::Duration::from_secs(8), async {
                    loop {
                        send(&mut stream, get()).await;
                        let state = reply(&mut stream).await.state.unwrap();
                        if state.convergence == SessionMcpConvergence::Blocked {
                            assert!(
                                serde_json::to_string(&state)
                                    .unwrap()
                                    .contains("mcp_refresh_failed")
                            );
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    }
                })
                .await
                .unwrap();
                // Removing the fault alone is not a retry input. The supervisor must remain suspended.
                fs::remove_file(root.join("fail-initialize")).unwrap();
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                assert_eq!(
                    fs::read_to_string(root.join("echo-agent.pids"))
                        .unwrap()
                        .lines()
                        .count(),
                    2
                );
                send(&mut stream, update(2, vec![target])).await;
                reply(&mut stream).await;
                current_healthy(&mut stream, 2, 0).await;
                assert_eq!(
                    fs::read_to_string(root.join("echo-agent.pids"))
                        .unwrap()
                        .lines()
                        .count(),
                    3
                );
                send(&mut stream, end()).await;
                while !matches!(
                    receive(&mut stream).await,
                    NodeToControllerMessage::AgentSessionEnded(_)
                ) {}
            });
        child.terminate();
    });
}
