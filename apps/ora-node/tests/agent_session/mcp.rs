//! SessionHost -> plugin transport -> real ACP initialize/session/new, with no secret artifacts.
use super::support::*;
use ora_node::{SessionHost, SessionWorkload};
use ora_node_db::{NodeDatabase, NodeIdentity, SessionCommandInput};
use ora_node_protocol::*;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;

const SECRET: &str = "m938-runtime-secret";

/// Installs both MCP shapes without saving any runtime settings on disk.
fn configure(fixture: &Fixture, load: bool, http: bool) -> Vec<SessionMcpBinding> {
    std::fs::write(
        fixture.package_root().join("mcp-capabilities.json"),
        json!({"load":load, "http":http}).to_string(),
    )
    .unwrap();
    let mut bindings = Vec::new();
    for (name, transport) in [
        (
            "acme.stdio",
            json!({"type":"stdio", "command":"assets/server",
            "args":[{"context":"workspace"}, {"setting":"argument", "prefix":"arg-"}],
            "env":{"TOKEN":{"setting":"token"}}}),
        ),
        (
            "acme.web",
            json!({"type":"http", "url":"https://mcp.example.test/mcp",
            "headers":{"Authorization":{"setting":"token", "prefix":"Bearer "}}}),
        ),
    ] {
        let path = fixture
            .home()
            .join("plugins")
            .join("installed")
            .join("official")
            .join(name)
            .join("1.0.0");
        std::fs::create_dir_all(path.join("assets")).unwrap();
        std::fs::write(path.join("orax.toml"), format!("resolver = 1\nidentifier = \"{name}\"\nkind = \"mcp\"\nversion = \"1.0.0\"\ndescription = \"test MCP\"\n")).unwrap();
        std::fs::write(path.join("assets").join("server"), "#!/bin/sh\nexit 0\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            path.join("assets").join("server"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        std::fs::write(path.join("assets").join("config.json"), json!({"schemaVersion":1,
            "settings":{
                "token":{"type":"string", "title":"Token", "description":"test", "required":true},
                "argument":{"type":"string", "title":"Argument", "description":"test", "default":"unused"}
            }, "transport":transport}).to_string()).unwrap();
        bindings.push(SessionMcpBinding {
            plugin_id: PluginId::new(format!("official/{name}")),
            plugin_version: PluginVersion::new("1.0.0"),
            settings: BTreeMap::from([
                ("token".into(), SettingValue::String(SECRET.into())),
                ("argument".into(), SettingValue::String(SECRET.into())),
            ]),
        });
    }
    bindings
}

/// Supplies the same live spec to durable admission and the production SessionHost.
fn input(mcp: Vec<SessionMcpBinding>) -> StartAgentSessionMessage {
    StartAgentSessionMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: OperationId::new(OPERATION),
        execution_id: ExecutionId::new(EXECUTION),
        payload: StartAgentSession {
            spec: AgentSessionSpec {
                mcp,
                node_id: NodeId::new("node-1"),
                agent_plugin_id: PluginId::new(PLUGIN_ID),
                agent_plugin_version: PluginVersion::new(PLUGIN_VERSION),
                checkout_execution_id: ExecutionId::new(CHECKOUT_EXECUTION),
                agent_configuration: None,
                model_binding_id: None,
                git_identity: identity(),
                initial_turn: turn("turn-1", "[mcp]"),
                prior_revision: None,
                workspace_directory: Vec::new(),
                trigger: None,
            },
        },
    }
}

/// Failure tests use the same input through the in-memory-ledger harness.
fn start(sessions: &Sessions, mcp: Vec<SessionMcpBinding>) {
    let input = input(mcp);
    sessions.start(input.operation_id, input.execution_id, input.payload.spec);
}

/// Searches actual log/history/package files, including binary or non-UTF-8 files.
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
                "secret persisted at {}",
                path.display()
            );
        }
    }
}

/// The fixture validates the complete session/new frame in memory, and replies with a safe marker.
#[test]
fn mcp_reaches_real_acp_session_new_without_persisting_or_logging_settings() {
    ora_logging::with_trace_logging(|| {
        for separate in [false, true] {
            let fixture = Fixture::new();
            let log_path = fixture.home().join("mcp-trace.log");
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
                        .block_on(async {
                            let bindings =
                                configure(&fixture, /*load*/ true, /*http*/ true);
                            let workload = if separate {
                                fixture.separate_workload()
                            } else {
                                SessionWorkload::Shared
                            };
                            if separate {
                                std::fs::write(
                                    fixture.package_root().join("mcp-command-view.json"),
                                    json!({
                                        "workload": fixture.workload_directory().canonicalize().unwrap(),
                                        "home": fixture.home().canonicalize().unwrap(),
                                    }).to_string(),
                                )
                                .unwrap();
                            }
                            let mut database = NodeDatabase::open(&fixture.home().join("ledger"), NodeIdentity::Require(NodeId::new("node-1"))).unwrap();
                            let input = input(bindings);
                            database.accept_session(&input).unwrap();
                            database.start_session(&input, &fixture.node().incarnation_id).unwrap();
                            let sessions = fixture.compose_with_ledger(PLUGIN_VERSION, workload,
                                /*model_proxy*/ None, database.session_journal().unwrap());
                            sessions.start(input.operation_id.clone(), input.execution_id.clone(), input.payload.spec);
                            until(|| {
                                database.pending_events().unwrap().iter().any(|event|
                                    matches!(event, NodeToControllerMessage::ThreadEvent(message)
                                        if super::field(&message.payload.record, &["type"]) == "turnEnded"))
                                    .then_some(())
                            }).await;
                            let events = serde_json::to_string(&database.pending_events().unwrap()).unwrap();
                            assert!(events.contains("MCP frame verified"), "fixture did not accept the ACP server list");
                            assert!(!events.contains(SECRET));
                            assert_eq!(fixture.leases.load(std::sync::atomic::Ordering::SeqCst), 3);
                            database.accept_session_command(&SessionCommandInput::EndSession(EndSessionMessage {
                                protocol_version: CURRENT_PROTOCOL_VERSION,
                                operation_id: input.operation_id.clone(), execution_id: input.execution_id.clone(),
                                payload: EndSession { node_id: NodeId::new("node-1"), command_id: CommandId::new("end"), reason: EndSessionReason::UserEnded },
                            })).unwrap();
                            sessions.command_arrived(&input.execution_id);
                            let ended = until(|| {
                                let record = database.find_session(&input.operation_id, &input.execution_id).unwrap().unwrap();
                                match record.state {
                                    ExecutionState::Completed(ExecutionResult::AgentSession(AgentSessionResult::AgentSessionEnded(ended))) => Some(ended),
                                    _ => None,
                                }
                            }).await;
                            assert_eq!(ended.reason, AgentSessionEndReason::UserEnded);
                            sessions.shutdown().await;
                            assert_eq!(fixture.leases.load(std::sync::atomic::Ordering::SeqCst), 0);
                            assert_no_secret_files(&fixture.home());
                            if separate {
                                assert_eq!(
                                    std::fs::read_dir(fixture.workload_directory())
                                        .unwrap()
                                        .count(),
                                    0
                                );
                            }
                        });
                },
            );
            let logs = std::fs::read_to_string(log_path).unwrap();
            assert!(logs.contains("session/new"));
            assert!(!logs.contains(SECRET));
        }
    });
}

/// Capability errors can initialize but never send session/new or any setting value to the agent.
#[test]
fn mcp_capability_failures_stop_before_session_new() {
    for (load, http, code, plugin_id, transport) in [
        (false, true, "mcp_load_session_missing", None, None),
        (
            true,
            false,
            "mcp_http_capability_missing",
            Some(PluginId::new("official/acme.web")),
            Some(SessionMcpTransport::Http),
        ),
    ] {
        let fixture = Fixture::new();
        let log_path = fixture.home().join("mcp-capability-trace.log");
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
                    .block_on(async {
                        let bindings = configure(&fixture, load, http);
                        let sessions = fixture.sessions(PLUGIN_VERSION);
                        start(&sessions, bindings);
                        let ended = fixture.ended().await;
                        sessions.shutdown().await;
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
                            std::fs::read_to_string(
                                fixture.package_root().join("echo-acp-methods")
                            )
                            .unwrap(),
                            "initialize\n"
                        );
                        assert_no_secret_files(&fixture.home());
                        assert_eq!(fixture.leases.load(std::sync::atomic::Ordering::SeqCst), 0);
                    });
            },
        );
        let logs = std::fs::read_to_string(log_path).unwrap();
        assert!(logs.contains(code));
        if code == "mcp_http_capability_missing" {
            assert!(logs.contains("official/acme.web"));
            assert!(logs.contains("transport=\"http\""));
        }
        assert!(!logs.contains(SECRET));
    }
}

/// Package and setting errors need no initialize handshake and start no agent process.
#[test]
fn mcp_preflight_failures_start_no_agent() {
    for code in ["mcp_plugin_missing", "mcp_setting_missing"] {
        let fixture = Fixture::new();
        let log_path = fixture.home().join("mcp-preflight-trace.log");
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
                    .block_on(async {
                        let mut bindings =
                            configure(&fixture, /*load*/ true, /*http*/ true);
                        if code == "mcp_plugin_missing" {
                            bindings[0].plugin_version = PluginVersion::new("2.0.0");
                        } else {
                            bindings[0].settings.remove("token");
                        }
                        let sessions = fixture.sessions(PLUGIN_VERSION);
                        start(&sessions, bindings);
                        let ended = fixture.ended().await;
                        sessions.shutdown().await;
                        let (setting_id, transport) = if code == "mcp_setting_missing" {
                            (Some("token".into()), Some(SessionMcpTransport::Stdio))
                        } else {
                            (None, None)
                        };
                        assert_eq!(
                            ended,
                            AgentSessionEnded {
                                node: fixture.node(),
                                reason: AgentSessionEndReason::AgentFailed,
                                detail: Some(code.into()),
                                plugin_id: Some(Box::new(PluginId::new("official/acme.stdio"))),
                                setting_id,
                                transport,
                                scheduled_result: None,
                                artifact_manifest: None,
                            }
                        );
                        assert!(!serde_json::to_string(&ended).unwrap().contains(SECRET));
                        assert_eq!(fixture.plugin_pids(), Vec::<u32>::new());
                    });
            },
        );
        let logs = std::fs::read_to_string(&log_path).unwrap();
        assert!(logs.contains("session MCP preflight failed"));
        assert!(logs.contains(code));
        assert!(logs.contains("official/acme.stdio"));
        if code == "mcp_setting_missing" {
            assert!(logs.contains("token"));
        }
        assert!(!logs.contains(SECRET));
        assert_no_secret_files(&fixture.home());
    }
}

/// An MCP session and a plain session share one checkout: each gets exactly its own MCP setup
/// (the plain sibling's `session/new` lists no server), and ending the MCP session releases its
/// package leases while the plain sibling keeps running.
#[test]
fn an_mcp_session_shares_a_checkout_with_a_plain_one() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let bindings = configure(&fixture, /*load*/ true, /*http*/ true);
                let mut database = NodeDatabase::open(
                    &fixture.home().join("ledger"),
                    NodeIdentity::Require(NodeId::new("node-1")),
                )
                .unwrap();
                let with_mcp = input(bindings);
                let mut plain = input(Vec::new());
                plain.operation_id = OperationId::new("plain-operation");
                plain.execution_id = ExecutionId::new("plain-execution");
                for session in [&with_mcp, &plain] {
                    database.accept_session(session).unwrap();
                    database
                        .start_session(session, &fixture.node().incarnation_id)
                        .unwrap();
                }
                let sessions = fixture.compose_with_ledger(
                    PLUGIN_VERSION,
                    SessionWorkload::Shared,
                    /*model_proxy*/ None,
                    database.session_journal().unwrap(),
                );
                for session in [&with_mcp, &plain] {
                    sessions.start(
                        session.operation_id.clone(),
                        session.execution_id.clone(),
                        session.payload.spec.clone(),
                    );
                }
                // Both initial turns say `[mcp]`, so each Agent reports on its own frame.
                let replies = until(|| {
                    let events = database.pending_events().unwrap();
                    let reply = |execution: &ExecutionId| {
                        events.iter().find_map(|event| match event {
                            NodeToControllerMessage::ThreadEvent(message)
                                if message.execution_id == *execution =>
                            {
                                let text = serde_json::to_string(&message.payload.record).ok()?;
                                ["MCP frame verified", "MCP frame invalid"]
                                    .into_iter()
                                    .find(|reply| text.contains(reply))
                            }
                            _ => None,
                        })
                    };
                    Some((reply(&with_mcp.execution_id)?, reply(&plain.execution_id)?))
                })
                .await;
                assert_eq!(replies, ("MCP frame verified", "MCP frame invalid"));
                // The MCP session holds its two server packages besides the Agent package of
                // each session.
                assert_eq!(fixture.leases.load(std::sync::atomic::Ordering::SeqCst), 4);

                let end = |session: &StartAgentSessionMessage| {
                    SessionCommandInput::EndSession(EndSessionMessage {
                        protocol_version: CURRENT_PROTOCOL_VERSION,
                        operation_id: session.operation_id.clone(),
                        execution_id: session.execution_id.clone(),
                        payload: EndSession {
                            node_id: NodeId::new("node-1"),
                            command_id: CommandId::new(format!(
                                "end-{}",
                                session.execution_id.as_str()
                            )),
                            reason: EndSessionReason::UserEnded,
                        },
                    })
                };
                let state = |database: &NodeDatabase, session: &StartAgentSessionMessage| {
                    database
                        .find_session(&session.operation_id, &session.execution_id)
                        .unwrap()
                        .unwrap()
                        .state
                };
                database.accept_session_command(&end(&with_mcp)).unwrap();
                sessions.command_arrived(&with_mcp.execution_id);
                until(|| {
                    matches!(state(&database, &with_mcp), ExecutionState::Completed(_))
                        .then_some(())
                })
                .await;
                until(|| {
                    (fixture.leases.load(std::sync::atomic::Ordering::SeqCst) == 1).then_some(())
                })
                .await;
                assert_eq!(state(&database, &plain), ExecutionState::Running);

                database.accept_session_command(&end(&plain)).unwrap();
                sessions.command_arrived(&plain.execution_id);
                until(|| {
                    matches!(state(&database, &plain), ExecutionState::Completed(_)).then_some(())
                })
                .await;
                sessions.shutdown().await;
                assert_eq!(fixture.leases.load(std::sync::atomic::Ordering::SeqCst), 0);
                assert_no_secret_files(&fixture.home());
            });
    });
}
